//! Background task runner: spawns `harness::run_task` on a tokio task,
//! forwarding `TaggedEvent`s through an mpsc channel back to the main loop.

use pulse_core::harness::{self, TaggedEvent};
use tokio::sync::mpsc::UnboundedSender;
use tokio::task::JoinHandle;

/// Spawn `run_task` on a tokio task. Events are sent through `tx`.
pub fn spawn_task(
    beat_id: i64,
    prompt: String,
    images: Vec<String>,
    tx: UnboundedSender<TaggedEvent>,
) -> JoinHandle<()> {
    tokio::spawn(async move {
        let mut callback = move |ev: TaggedEvent| {
            let _ = tx.send(ev);
        };
        let _ = harness::run_task(beat_id, prompt, images, &mut callback).await;
    })
}
