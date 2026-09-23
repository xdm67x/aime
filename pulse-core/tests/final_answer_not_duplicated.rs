//! Regression: the final answer must not be duplicated — neither live (the
//! same text emitted as both a streaming delta and a step) nor persisted
//! (the same reply stored as two adjacent assistant entries).

use pulse_core::{beats, db, harness, workflows::BASE};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

fn sse_for(entry: &Value) -> String {
    let mut out = String::new();
    let chunk = |delta: Value, fr: Option<&str>| {
        let mut c = json!({"choices":[{"delta": delta}]});
        if let Some(f) = fr {
            c["choices"][0]["finish_reason"] = json!(f);
        }
        format!("data: {}\n\n", c)
    };
    if let Some(fr) = entry.get("finish_reason").and_then(|f| f.as_str()) {
        if !entry["content"].as_str().unwrap_or("").is_empty() {
            out.push_str(&chunk(json!({"content": entry["content"]}), None));
        }
        out.push_str(&chunk(json!({}), Some(fr)));
    } else {
        // tool-call round: name + arguments streamed as fragments
        out.push_str(&chunk(json!({"content": entry["narration"]}), None));
        out.push_str(&chunk(
            json!({"tool_calls":[{"index":0,"id":"c1","function":{
            "name": entry["tool"], "arguments": entry["args"]}}]}),
            None,
        ));
        out.push_str(&chunk(json!({}), Some("tool_calls")));
    }
    out.push_str(
        "data: {\"choices\":[],\"usage\":{\"prompt_tokens\":10,\"completion_tokens\":5}}\n\n",
    );
    out.push_str("data: [DONE]\n\n");
    out
}

fn spawn_mock(script: Vec<Value>) -> (String, Arc<AtomicUsize>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let req_no = Arc::new(AtomicUsize::new(0));
    let n = req_no.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut s = stream.unwrap();
            // read the full request: headers, then Content-Length body bytes
            let mut raw: Vec<u8> = Vec::new();
            let mut buf = [0u8; 16384];
            let header_end = |v: &[u8]| v.windows(4).position(|w| w == b"\r\n\r\n");
            while header_end(&raw).is_none() {
                let m = s.read(&mut buf).unwrap_or(0);
                if m == 0 {
                    break;
                }
                raw.extend_from_slice(&buf[..m]);
            }
            let Some(h) = header_end(&raw) else { continue };
            let head = String::from_utf8_lossy(&raw[..h]).to_string();
            let content_len: usize = head
                .lines()
                .find_map(|l| {
                    l.to_ascii_lowercase()
                        .strip_prefix("content-length:")
                        .and_then(|v| v.trim().parse().ok())
                })
                .unwrap_or(0);
            while raw.len() < h + 4 + content_len {
                let m = s.read(&mut buf).unwrap_or(0);
                if m == 0 {
                    break;
                }
                raw.extend_from_slice(&buf[..m]);
            }
            let raw = String::from_utf8_lossy(&raw).to_string();
            let mut reply = |payload: String| {
                let resp = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                    payload.len(),
                    payload
                );
                s.write_all(resp.as_bytes()).unwrap();
                s.flush().unwrap();
            };
            // model-list fetches (context checks, usage pricing) must not
            // consume scripted chat rounds
            if raw.starts_with("GET") {
                reply(r#"{"data":[]}"#.to_string());
                continue;
            }
            let i = n.fetch_add(1, Ordering::SeqCst);
            let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
            let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let model = v["model"].as_str().unwrap_or("").to_string();
            eprintln!(
                "--- request #{i}: model={model} tools={}",
                v.get("tools").is_some()
            );
            let payload = {
                let entry = script
                    .get(i)
                    .cloned()
                    .unwrap_or(json!({"content":"(script exhausted)","finish_reason":"stop"}));
                sse_for(&entry)
            };
            reply(payload);
        }
    });
    (format!("http://{addr}/v1"), req_no)
}

fn setup_home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", dir.path());
    dir
}

fn configure(url: &str) {
    db::set_setting("litellm_base_url", url).unwrap();
    db::set_setting("litellm_api_key", "test").unwrap();
    // Plain prompts run through the base workflow: one step that hands the
    // user's message to the worker model.
    pulse_core::workflows::create(BASE, "", "LiteLLM - worker", "{{prompt}}").unwrap();
}

fn new_beat() -> i64 {
    let proj_dir = tempfile::tempdir().unwrap();
    let proj = pulse_core::projects::add_project(proj_dir.path().to_str().unwrap()).unwrap();
    let id = beats::create_beat("repro", "", Some(proj.id)).unwrap().id;
    std::mem::forget(proj_dir);
    id
}

#[tokio::test]
async fn final_answer_is_not_duplicated() {
    // --- plain prompt (base workflow): plain-text answer, reminder,
    //     task_complete restates it ---
    let _home = setup_home();
    let script = vec![
        json!({"content": "The answer is 42.", "finish_reason": "stop"}),
        json!({"narration": "", "tool": "task_complete", "args": "{\"summary\":\"The answer is 42.\"}"}),
    ];
    let (url, _n) = spawn_mock(script);
    configure(&url);
    let beat_id = new_beat();

    let mut events: Vec<Value> = vec![];
    let r = harness::run_task(
        beat_id,
        "what is the answer?".to_string(),
        vec![],
        &mut |ev| {
            events.push(serde_json::to_value(&ev).unwrap());
        },
    )
    .await
    .unwrap();
    assert_eq!(r.answer, "The answer is 42.");
    assert_eq!(r.workflow, BASE);
    assert_eq!(r.model, "LiteLLM - worker");

    // live: the answer may stream as deltas, but no step may repeat it
    let answer_steps = events
        .iter()
        .filter(|e| e["type"] == "step")
        .filter(|e| e["text"].as_str() == Some("The answer is 42."))
        .count();
    assert_eq!(answer_steps, 0, "answer must not be re-emitted as a step");

    // persisted: exactly one assistant entry holds the answer
    let persisted = beats::get_beat_messages(beat_id).unwrap();
    let answer_entries = persisted
        .iter()
        .filter(|m| m["role"] == "assistant" && m["content"].as_str() == Some("The answer is 42."))
        .count();
    assert_eq!(answer_entries, 1, "answer must be persisted exactly once");

    // --- end_turn reply restated by task_complete after more tool work ---
    // The model ends its turn with the plain final answer (finish_reason
    // "end_turn"), gets the reminder, does one more tool round, then calls
    // `task_complete` restating the same answer. A tool entry sits between
    // the reply and the summary, and the restatement varies whitespace — the
    // result must still appear exactly once.
    let _home = setup_home();
    let script = vec![
        json!({"narration": "Let me check.", "tool": "bash", "args": "{\"command\":\"echo hi\"}"}),
        json!({"content": "The answer is 42.", "finish_reason": "end_turn"}),
        json!({"narration": "", "tool": "bash", "args": "{\"command\":\"echo done\"}"}),
        json!({"narration": "", "tool": "task_complete", "args": "{\"summary\":\"The   answer is\\n42.\"}"}),
    ];
    let (url, _n) = spawn_mock(script);
    configure(&url);
    let beat_id = new_beat();

    let mut events: Vec<Value> = vec![];
    let r = harness::run_task(beat_id, "what is the answer?".to_string(), vec![], &mut |ev| {
        events.push(serde_json::to_value(&ev).unwrap());
    })
    .await
    .unwrap();
    assert_eq!(r.answer, "The answer is 42.");

    // live: the answer streamed as deltas; the restated summary must not be
    // emitted as a step on top of it
    let answer_steps = events
        .iter()
        .filter(|e| e["type"] == "step")
        .filter(|e| e["text"].as_str().map(|t| t.split_whitespace().collect::<Vec<_>>().join(" "))
            == Some("The answer is 42.".to_string()))
        .count();
    assert_eq!(answer_steps, 0, "restated summary must not be emitted as a step");

    // persisted: the end_turn reply IS the answer — exactly one copy
    let persisted = beats::get_beat_messages(beat_id).unwrap();
    let answer_entries = persisted
        .iter()
        .filter(|m| m["role"] == "assistant"
            && m["content"].as_str().map(|c| c.split_whitespace().collect::<Vec<_>>().join(" "))
                == Some("The answer is 42.".to_string()))
        .count();
    assert_eq!(answer_entries, 1, "answer must be persisted exactly once");

    // --- task_complete restating the narration streamed on its own round ---
    // Some models stream the final text next to the `task_complete` call;
    // the summary must not be emitted or persisted as a second copy.
    let _home = setup_home();
    let script = vec![
        json!({"narration": "Let me check.", "tool": "bash", "args": "{\"command\":\"echo hi\"}"}),
        json!({"narration": "Here is the result.", "tool": "task_complete",
               "args": "{\"summary\":\"Here is the result.\"}"}),
    ];
    let (url, _n) = spawn_mock(script);
    configure(&url);
    let beat_id = new_beat();

    let mut events: Vec<Value> = vec![];
    let r = harness::run_task(beat_id, "do it".to_string(), vec![], &mut |ev| {
        events.push(serde_json::to_value(&ev).unwrap());
    })
    .await
    .unwrap();
    assert_eq!(r.answer, "Here is the result.");

    // live: the text streamed as deltas — no step may repeat it
    let result_steps = events
        .iter()
        .filter(|e| e["type"] == "step")
        .filter(|e| e["text"].as_str() == Some("Here is the result."))
        .count();
    assert_eq!(result_steps, 0, "streamed narration must not be re-emitted as a step");

    // persisted: exactly one assistant entry holds the answer
    let persisted = beats::get_beat_messages(beat_id).unwrap();
    let result_entries = persisted
        .iter()
        .filter(|m| m["role"] == "assistant" && m["content"].as_str() == Some("Here is the result."))
        .count();
    assert_eq!(result_entries, 1, "answer must be persisted exactly once");
}
