import type {
  Beat,
  Model,
  ModelConfig,
  Project,
  TaskEvent,
  TaskResult,
  UsageTotal,
  Workflow,
} from "./native";

export type { Beat, Model, ModelConfig, Project, TaskEvent, TaskResult, UsageTotal, Workflow };

export type WebviewToHost =
  | { kind: "ready" }
  | { kind: "run-task"; beatId: number; prompt: string; images: string[] }
  | { kind: "cancel-task"; beatId: number }
  | { kind: "list-beats" }
  | { kind: "create-beat"; name: string; description: string; projectId: number | null }
  | { kind: "archive-beat"; beatId: number; archived: boolean }
  | { kind: "delete-beat"; beatId: number }
  | { kind: "get-beat-messages"; beatId: number }
  | { kind: "list-projects" }
  | { kind: "add-project"; path: string }
  | { kind: "remove-project"; projectId: number }
  | { kind: "list-models" }
  | { kind: "get-model-config" }
  | { kind: "save-model-config"; config: ModelConfig }
  | { kind: "list-workflows" }
  | { kind: "save-workflows"; workflows: Workflow[] }
  | { kind: "set-default-workflow"; id: string }
  | { kind: "run-workflow"; beatId: number; workflowId: string; prompt: string; images: string[] }
  | { kind: "save-api-key"; provider: string; key: string }
  | { kind: "get-api-key"; provider: string }
  | { kind: "beat-usage-totals"; beatId: number }
  | { kind: "pick-folder" };

export type HostToWebview =
  | { kind: "task-event"; beatId: number; ev: TaskEvent }
  | { kind: "task-result"; beatId: number; result: TaskResult }
  | { kind: "task-error"; beatId: number; error: string }
  | { kind: "beats"; beats: Beat[] }
  | { kind: "beat-messages"; beatId: number; messages: unknown[] }
  | { kind: "projects"; projects: Project[] }
  | { kind: "models"; models: Model[] }
  | { kind: "model-config"; config: ModelConfig }
  | { kind: "workflows"; workflows: Workflow[]; defaultId: string }
  | { kind: "api-key"; provider: string; key: string | null }
  | { kind: "usage-totals"; beatId: number; totals: UsageTotal[] }
  | { kind: "folder-picked"; path: string | null }
  | { kind: "toast"; text: string };
