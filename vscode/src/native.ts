/**
 * Native harness bindings: the napi-rs addon wrapping pulse-core (the Rust
 * engine shared with the Tauri app). Types here are the source of truth for
 * the webview; everything crossing the webview boundary is plain JSON.
 *
 * The addon is resolved lazily: the workspace `pulse-node` package first
 * (development, tests), then the copy vendored inside the .vsix under
 * `native/` (`vsce package --no-dependencies` strips `node_modules`).
 */
import type * as PulseNode from "pulse-node";

export type NativeModule = typeof PulseNode;

declare const require: (id: string) => unknown;

let resolved: NativeModule | null = null;

function loadAddon(): NativeModule {
  const candidates = ["pulse-node", "../native/pulse-node"];
  const errors: unknown[] = [];
  for (const id of candidates) {
    try {
      return require(id) as NativeModule;
    } catch (err) {
      errors.push(err);
    }
  }
  throw new Error(
    `Failed to load the pulse-node native addon (tried ${candidates.join(", ")}): ${errors
      .map((e) => (e instanceof Error ? e.message : String(e)))
      .join(" | ")}`,
  );
}

/**
 * Returns the pulse-node addon, loading it on first use from the workspace
 * package or the vendored copy shipped inside the .vsix.
 */
export function requireNative(): NativeModule {
  resolved ??= loadAddon();
  return resolved;
}

export type {
  Beat,
  Model,
  ModelConfig,
  Project,
  SkillInfo,
  TaskResult,
  UsageTotal,
} from "pulse-node";

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
): Promise<PulseNode.TaskResult> {
  return requireNative().runTask(beatId, prompt, images, (ev) => {
    onEvent(ev as TaggedEvent);
  });
}

export function addProject(path: string): PulseNode.Project {
  return requireNative().addProject(path);
}

export function removeProject(projectId: number): void {
  requireNative().removeProject(projectId);
}

export function cancelCurrent(beatId: number): void {
  requireNative().cancelCurrent(beatId);
}

export function createBeat(
  name: string,
  description: string,
  projectId?: number | null,
): PulseNode.Beat {
  return requireNative().createBeat(name, description, projectId);
}

export function deleteBeat(id: number): string {
  return requireNative().deleteBeat(id);
}

export function discoverSkills(): PulseNode.SkillInfo[] {
  return requireNative().discoverSkills();
}

export function executeTool(name: string, args: string, cwd?: string | null): Promise<string> {
  return requireNative().executeTool(name, args, cwd);
}

export function getApiKey(provider: string): string | null {
  return requireNative().getApiKey(provider);
}

export function getBeatMessages(id: number): Array<Record<string, unknown>> {
  return requireNative().getBeatMessages(id);
}

export function getModelConfig(): PulseNode.ModelConfig {
  return requireNative().getModelConfig();
}

export function listBeats(): PulseNode.Beat[] {
  return requireNative().listBeats();
}

export function listModels(): Promise<PulseNode.Model[]> {
  return requireNative().listModels();
}

export function listProjects(): PulseNode.Project[] {
  return requireNative().listProjects();
}

export function saveApiKey(provider: string, key: string): void {
  requireNative().saveApiKey(provider, key);
}

export function saveModelConfig(cfg: PulseNode.ModelConfig): void {
  requireNative().saveModelConfig(cfg);
}

export function setBeatArchived(id: number, archived: boolean): void {
  requireNative().setBeatArchived(id, archived);
}

export function startModelsRefresh(): void {
  requireNative().startModelsRefresh();
}

export function stripDiff(result: string): string {
  return requireNative().stripDiff(result);
}

export function unifiedDiff(before: string, after: string, maxLines: number): string {
  return requireNative().unifiedDiff(before, after, maxLines);
}

export function usageTotals(beatId: number): PulseNode.UsageTotal[] {
  return requireNative().usageTotals(beatId);
}
