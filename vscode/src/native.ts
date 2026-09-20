/**
 * Native harness bindings: the napi-rs addon wrapping pulse-core (the Rust
 * engine shared with the Tauri app). Types here are the source of truth for
 * the webview; everything crossing the webview boundary is plain JSON.
 */
import * as native from "pulse-node";

export type { Beat, Model, ModelConfig, Project, TaskResult, UsageTotal } from "pulse-node";

export type TaskEvent =
  | { type: "start"; model: string; tier: string }
  | { type: "delta"; text: string }
  | { type: "tool"; tool: string; arguments: string; result: string; error: boolean }
  | { type: "step"; text: string };

/** Live event streamed from a running task, tagged with its beat. */
export type TaggedEvent = TaskEvent & { beatId: number };

export function runTask(
  beatId: number,
  prompt: string,
  images: string[],
  onEvent: (ev: TaggedEvent) => void,
): Promise<native.TaskResult> {
  return native.runTask(beatId, prompt, images, (ev) => {
    onEvent(ev as TaggedEvent);
  });
}

export function addProject(path: string): native.Project {
  return native.addProject(path);
}

export function removeProject(projectId: number): void {
  native.removeProject(projectId);
}

export {
  cancelCurrent,
  createBeat,
  deleteBeat,
  discoverSkills,
  executeTool,
  getApiKey,
  getBeatMessages,
  getModelConfig,
  listBeats,
  listModels,
  listProjects,
  saveApiKey,
  saveModelConfig,
  setBeatArchived,
  startModelsRefresh,
  stripDiff,
  unifiedDiff,
  usageTotals,
} from "pulse-node";
