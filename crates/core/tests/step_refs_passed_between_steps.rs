//! Steps can reference earlier steps' results with `{{steps.<name>}}`
//! placeholders: the workflow engine fills them with the finished step's
//! answer before the next step runs. The mock provider scripts both steps'
//! replies and captures the request bodies, so the test asserts the second
//! step's prompt literally contains the first step's answer.

use aime_core::{beats, db, projects, workflows};
use serde_json::{json, Value};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::sync::{Arc, Mutex};

fn sse_for(entry: &Value) -> String {
    let chunk = |delta: Value, fr: Option<&str>| {
        let mut c = json!({"choices":[{"delta": delta}]});
        if let Some(f) = fr {
            c["choices"][0]["finish_reason"] = json!(f);
        }
        format!("data: {}\n\n", c)
    };
    let mut out = String::new();
    if let Some(fr) = entry.get("finish_reason").and_then(|f| f.as_str()) {
        if !entry["content"].as_str().unwrap_or("").is_empty() {
            out.push_str(&chunk(json!({"content": entry["content"]}), None));
        }
        out.push_str(&chunk(json!({}), Some(fr)));
    } else {
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
    out.push_str("data: \n\n");
    out
}

/// Mock provider capturing each request's messages, replying with the
/// scripted SSE round.
fn spawn_mock(script: Vec<Value>) -> (String, Arc<Mutex<Vec<Vec<Value>>>>) {
    let listener = TcpListener::bind("127.0.0.1:0").unwrap();
    let addr = listener.local_addr().unwrap();
    let seen = Arc::new(Mutex::new(vec![]));
    let seen_c = seen.clone();
    let script = Arc::new(script);
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let mut s = stream.unwrap();
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
            if raw.starts_with("GET") {
                reply(r#"{"data":[]}"#.to_string());
                continue;
            }
            let body = raw.split("\r\n\r\n").nth(1).unwrap_or("").to_string();
            let v: Value = serde_json::from_str(&body).unwrap_or(Value::Null);
            // Non-streaming requests (the per-step history summarization)
            // get a plain JSON completion; streaming rounds get the scripted
            // SSE and are recorded so the test can inspect step prompts.
            if v["stream"].as_bool() != Some(true) {
                reply(
                    r#"{"choices":[{"message":{"role":"assistant","content":"prior context"}}],"usage":{"prompt_tokens":10,"completion_tokens":5}}"#
                        .to_string(),
                );
                continue;
            }
            seen_c
                .lock()
                .unwrap()
                .push(v["messages"].as_array().cloned().unwrap_or_default());
            let i = seen_c.lock().unwrap().len() - 1;
            let payload = sse_for(
                &script
                    .get(i)
                    .cloned()
                    .unwrap_or(json!({"content":"(script exhausted)","finish_reason":"stop"})),
            );
            reply(payload);
        }
    });
    (format!("http://{addr}/v1"), seen)
}

fn setup_home() -> tempfile::TempDir {
    let dir = tempfile::tempdir().unwrap();
    std::env::set_var("HOME", dir.path());
    dir
}

#[tokio::test]
async fn step_results_flow_into_later_steps_prompts() {
    let _home = setup_home();
    // step 1 ends its turn with a plain reply, gets the reminder, then
    // completes; step 2 completes immediately. Both answers are distinctive
    // so the substitution is unambiguous.
    let script = vec![
        json!({"content": "found 3 failing tests", "finish_reason": "end_turn"}),
        json!({"tool": "task_complete", "args": "{\"summary\":\"found 3 failing tests\"}"}),
        json!({"tool": "task_complete", "args": "{\"summary\":\"all 3 fixed\"}"}),
    ];
    let (url, seen) = spawn_mock(script);
    db::set_setting("litellm_base_url", &url).unwrap();
    db::set_setting("litellm_api_key", "test").unwrap();

    let wf: workflows::Workflow = serde_yaml::from_str(
        "
name: fix
model: worker
steps:
  - name: find
    prompt: Find what breaks.
  - name: fix
    prompt: \"Fix what {{steps.find}} broke.\"
",
    )
    .unwrap();
    assert!(wf.validate().is_ok());

    let proj_dir = tempfile::tempdir().unwrap();
    let proj = projects::add_project(proj_dir.path().to_str().unwrap()).unwrap();
    let beat = beats::create_beat("repro", "", Some(proj.id)).unwrap();
    std::mem::forget(proj_dir);

    let result = workflows::run(beat.id, &wf, None, &[], &mut |_| {})
        .await
        .unwrap();
    assert_eq!(result.steps.len(), 2);
    assert_eq!(result.steps[0].answer, "found 3 failing tests");
    assert_eq!(result.steps[1].answer, "all 3 fixed");

    // the second step's outgoing prompt must carry the first step's answer
    let seen = seen.lock().unwrap();
    let second_round_user = seen
        .iter()
        .flat_map(|msgs| msgs.iter())
        .filter(|m| m["role"] == "user")
        .map(|m| m["content"].as_str().unwrap_or(""))
        .find(|c| c.contains("Fix what"))
        .expect("second step's user prompt reached the provider");
    assert!(
        second_round_user.contains("found 3 failing tests"),
        "second step prompt must contain the first step's answer, got: {second_round_user}"
    );
    assert!(
        !second_round_user.contains("{{steps.find}}"),
        "placeholder must be substituted, got: {second_round_user}"
    );
}
