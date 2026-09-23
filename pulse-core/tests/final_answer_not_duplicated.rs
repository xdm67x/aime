//! Regression: the final answer must not be duplicated — neither live (the
//! same text emitted as both a streaming delta and a step) nor persisted
//! (the same reply stored as two adjacent assistant entries).

use pulse_core::{beats, db, harness};
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

fn spawn_mock(tier: &str, script: Vec<Value>) -> (String, Arc<AtomicUsize>) {
    let tier = tier.to_string();
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let req_no = Arc::new(AtomicUsize::new(0));
    let n = req_no.clone();
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut s = stream.unwrap();
            let i = n.fetch_add(1, Ordering::SeqCst);
            let mut buf = vec![0u8; 65536];
            let m = s.read(&mut buf).unwrap_or(0);
            let raw = String::from_utf8_lossy(&buf[..m]).to_string();
            let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
            let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            let model = v["model"].as_str().unwrap_or("").to_string();
            eprintln!(
                "--- request #{i}: model={model} tools={}",
                v.get("tools").is_some()
            );
            let payload = if model.contains("classifier") {
                // non-streaming classify response
                json!({"choices":[{"message":{"content":serde_json::json!({"tier": tier.to_string(), "n": 2}).to_string()},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}).to_string()
            } else {
                let entry = script
                    .get(i.saturating_sub(1))
                    .cloned()
                    .unwrap_or(json!({"content":"(script exhausted)","finish_reason":"stop"}));
                sse_for(&entry)
            };
            let resp = format!(
                "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{}",
                payload.len(),
                payload
            );
            s.write_all(resp.as_bytes()).unwrap();
            s.flush().unwrap();
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
    db::set_setting("model_classifier", "LiteLLM - classifier").unwrap();
    db::set_setting("model_base", "LiteLLM - worker").unwrap();
    db::set_setting("model_low", "LiteLLM - low").unwrap();
    db::set_setting("model_high", "LiteLLM - worker").unwrap();
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
    // --- base tier: plain-text answer, reminder, task_complete restates it ---
    let _home = setup_home();
    let script = vec![
        json!({"content": "The answer is 42.", "finish_reason": "stop"}),
        json!({"narration": "", "tool": "task_complete", "args": "{\"summary\":\"The answer is 42.\"}"}),
    ];
    let (url, _n) = spawn_mock("base", script);
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

    // --- high tier: the task_complete draft must not be emitted/persisted twice ---
    let _home = setup_home();
    let script = vec![
        json!({"narration": "", "tool": "task_complete", "args": "{\"summary\":\"DRAFT ANSWER\"}"}),
        json!({"content": "REFINED ANSWER", "finish_reason": "stop"}),
    ];
    let (url, _n) = spawn_mock("high", script);
    configure(&url);
    let beat_id = new_beat();

    let mut events: Vec<Value> = vec![];
    let r = harness::run_task(beat_id, "do it well".to_string(), vec![], &mut |ev| {
        events.push(serde_json::to_value(&ev).unwrap());
    })
    .await
    .unwrap();
    assert_eq!(r.answer, "REFINED ANSWER");

    // the draft shows up once as a step, not twice
    let draft_steps = events
        .iter()
        .filter(|e| e["type"] == "step")
        .filter(|e| e["text"].as_str() == Some("DRAFT ANSWER"))
        .count();
    assert_eq!(
        draft_steps, 1,
        "draft must be emitted as a step exactly once"
    );

    // and is persisted once, before the refined answer (also persisted once)
    let persisted = beats::get_beat_messages(beat_id).unwrap();
    let draft_entries = persisted
        .iter()
        .filter(|m| m["role"] == "assistant" && m["content"].as_str() == Some("DRAFT ANSWER"))
        .count();
    assert_eq!(draft_entries, 1, "draft must be persisted exactly once");
    let refined_entries = persisted
        .iter()
        .filter(|m| m["role"] == "assistant" && m["content"].as_str() == Some("REFINED ANSWER"))
        .count();
    assert_eq!(refined_entries, 1, "refined answer must be persisted once");
}
