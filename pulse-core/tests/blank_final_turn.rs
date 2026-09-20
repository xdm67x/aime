//! End-to-end repro: drive harness::run_task against a local mock
//! OpenAI-compatible server and dump the event sequence the UI would see.

use pulse_core::{beats, db, harness};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;

/// Scenario steps: each chat request to the worker model is answered by the
/// next script entry. A script entry is either plain text (finish_reason
/// "stop") or ("text", finish_reason) or a tool call.
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
                json!({"choices":[{"message":{"content":"{\"tier\": \"base\", \"n\": 2}"},"finish_reason":"stop"}],"usage":{"prompt_tokens":1,"completion_tokens":1}}).to_string()
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

#[tokio::test]
async fn blank_final_turn_never_ends_run_silently() {
    let _home = setup_home();
    let script = vec![
        // round 1: tool call (narration + bash)
        json!({"narration": "Let me check.", "tool": "bash", "args": "{\"command\":\"echo hi\"}"}),
        // round 2: the model's final answer — but the provider marked it
        // tool_calls while no tool-call fragments were parseable
        json!({"content": "Here is the full answer with details", "finish_reason": "tool_calls"}),
        // round 3: continuation — model has nothing left to say (blank turn
        // with finish "stop": the exact shape that used to end the run with
        // no visible message)
        json!({"content": "", "finish_reason": "stop"}),
        // round 4: after the reminder, the model signals completion properly
        // via `task_complete` with its final answer
        json!({"narration": "", "tool": "task_complete",
               "args": "{\"summary\":\"Here is the full answer with details — done\"}"}),
    ];
    let (url, _n) = spawn_mock(script);
    db::set_setting("litellm_base_url", &url).unwrap();
    db::set_setting("litellm_api_key", "test").unwrap();
    db::set_setting("model_classifier", "LiteLLM - classifier").unwrap();
    db::set_setting("model_base", "LiteLLM - worker").unwrap();
    db::set_setting("model_low", "LiteLLM - low").unwrap();
    db::set_setting("model_high", "LiteLLM - high").unwrap();

    let proj_dir = tempfile::tempdir().unwrap();
    let proj = pulse_core::projects::add_project(proj_dir.path().to_str().unwrap()).unwrap();
    let beat = beats::create_beat("repro", "", Some(proj.id)).unwrap();
    let mut events: Vec<Value> = vec![];
    let result = harness::run_task(beat.id, "do a thing".to_string(), &mut |ev| {
        events.push(serde_json::to_value(&ev).unwrap());
    })
    .await;
    println!("\n=== EVENTS ===");
    for e in &events {
        println!("{}", serde_json::to_string_pretty(e).unwrap());
    }
    println!(
        "=== RESULT ===\n{}",
        serde_json::to_string_pretty(&result).unwrap()
    );
    println!("=== PERSISTED ===");
    for m in beats::get_beat_messages(beat.id).unwrap() {
        println!(
            "{}: {}",
            m["role"],
            m["content"].as_str().unwrap_or("(non-str)")
        );
    }

    // the run must not finish with a blank answer: the truncated text is
    // the fallback, and the last visible step is non-empty
    let r = result.unwrap();
    assert_eq!(r.answer, "Here is the full answer with details — done");
    let steps: Vec<&str> = events
        .iter()
        .filter(|e| e["type"] == "step")
        .filter_map(|e| e["text"].as_str())
        .collect();
    assert!(steps.last().map(|t| !t.trim().is_empty()).unwrap_or(false));
    // transcript must not carry the blank turn or a duplicated fallback
    let persisted = beats::get_beat_messages(beat.id).unwrap();
    let last = persisted.last().unwrap();
    assert_eq!(last["role"], "assistant");
    assert_eq!(
        last["content"],
        "Here is the full answer with details — done"
    );
    assert_ne!(persisted[persisted.len() - 2]["content"], r.answer);
}
