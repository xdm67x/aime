import * as vscode from "vscode";
import type { HostToWebview, WebviewToHost } from "./protocol";
import * as native from "./native";
import { buildWebviewHtml } from "./webview-html";

export class PulsePanel {
  public static current: PulsePanel | undefined;

  private static readonly viewType = "pulse.harness";
  private readonly panel: vscode.WebviewPanel;
  private disposables: vscode.Disposable[] = [];
  private running = new Set<number>();

  public static createOrShow(extensionUri: vscode.Uri): void {
    if (PulsePanel.current) {
      PulsePanel.current.panel.reveal();
      return;
    }
    const panel = vscode.window.createWebviewPanel(
      PulsePanel.viewType,
      "Pulse",
      vscode.ViewColumn.Beside,
      {
        enableScripts: true,
        localResourceRoots: [vscode.Uri.joinPath(extensionUri, "webview-dist")],
        retainContextWhenHidden: true,
      },
    );
    new PulsePanel(panel, extensionUri);
  }

  private constructor(panel: vscode.WebviewPanel, extensionUri: vscode.Uri) {
    this.panel = panel;
    const webview = panel.webview;
    const dist = vscode.Uri.joinPath(extensionUri, "webview-dist");
    webview.html = buildWebviewHtml({
      cspSource: webview.cspSource,
      scriptUri: String(webview.asWebviewUri(vscode.Uri.joinPath(dist, "index.js"))),
      styleUri: String(webview.asWebviewUri(vscode.Uri.joinPath(dist, "index.css"))),
      nonce: getNonce(),
    });
    webview.onDidReceiveMessage(
      (m: WebviewToHost) => this.onMessage(m),
      undefined,
      this.disposables,
    );
    panel.onDidDispose(() => this.dispose(), undefined, this.disposables);
    PulsePanel.current = this;
  }

  private post(msg: HostToWebview): void {
    void this.panel.webview.postMessage(msg);
  }

  private async onMessage(msg: WebviewToHost): Promise<void> {
    try {
      switch (msg.kind) {
        case "ready":
          native.startModelsRefresh();
          this.post({ kind: "beats", beats: native.listBeats() });
          this.post({ kind: "projects", projects: native.listProjects() });
          this.post({ kind: "model-config", config: native.getModelConfig() });
          this.post({
            kind: "workflows",
            workflows: native.listWorkflows(),
            defaultId: native.getDefaultWorkflow(),
          });
          try {
            this.post({ kind: "models", models: await native.listModels() });
          } catch (e) {
            this.post({ kind: "toast", text: `Models unavailable: ${e}` });
          }
          return;
        case "run-task":
          this.startTask(msg.beatId, msg.prompt, msg.images);
          return;
        case "cancel-task":
          native.cancelCurrent(msg.beatId);
          return;
        case "list-beats":
          this.post({ kind: "beats", beats: native.listBeats() });
          return;
        case "create-beat": {
          const beat = native.createBeat(msg.name, msg.description, msg.projectId);
          this.post({ kind: "beats", beats: native.listBeats() });
          this.post({ kind: "toast", text: `Beat “${beat.name}” created` });
          return;
        }
        case "archive-beat":
          native.setBeatArchived(msg.beatId, msg.archived);
          this.post({ kind: "beats", beats: native.listBeats() });
          return;
        case "delete-beat":
          this.post({ kind: "toast", text: native.deleteBeat(msg.beatId) });
          this.post({ kind: "beats", beats: native.listBeats() });
          return;
        case "get-beat-messages":
          this.post({
            kind: "beat-messages",
            beatId: msg.beatId,
            messages: native.getBeatMessages(msg.beatId),
          });
          return;
        case "list-projects":
          this.post({ kind: "projects", projects: native.listProjects() });
          return;
        case "add-project":
          native.addProject(msg.path);
          this.post({ kind: "projects", projects: native.listProjects() });
          return;
        case "remove-project":
          native.removeProject(msg.projectId);
          this.post({ kind: "projects", projects: native.listProjects() });
          return;
        case "list-models":
          this.post({ kind: "models", models: await native.listModels() });
          return;
        case "get-model-config":
          this.post({ kind: "model-config", config: native.getModelConfig() });
          return;
        case "save-model-config":
          native.saveModelConfig(msg.config);
          this.post({ kind: "toast", text: "Model slots saved" });
          return;
        case "list-workflows":
          this.post({
            kind: "workflows",
            workflows: native.listWorkflows(),
            defaultId: native.getDefaultWorkflow(),
          });
          return;
        case "save-workflows":
          native.saveWorkflows(msg.workflows);
          this.post({
            kind: "workflows",
            workflows: native.listWorkflows(),
            defaultId: native.getDefaultWorkflow(),
          });
          this.post({ kind: "toast", text: "Workflows saved" });
          return;
        case "set-default-workflow":
          native.setDefaultWorkflow(msg.id);
          this.post({
            kind: "workflows",
            workflows: native.listWorkflows(),
            defaultId: native.getDefaultWorkflow(),
          });
          return;
        case "run-workflow":
          this.startWorkflow(msg.beatId, msg.workflowId, msg.prompt, msg.images);
          return;
        case "save-api-key":
          native.saveApiKey(msg.provider, msg.key);
          this.post({ kind: "toast", text: "API key saved" });
          return;
        case "get-api-key":
          this.post({
            kind: "api-key",
            provider: msg.provider,
            key: native.getApiKey(msg.provider),
          });
          return;
        case "beat-usage-totals":
          this.post({
            kind: "usage-totals",
            beatId: msg.beatId,
            totals: native.usageTotals(msg.beatId),
          });
          return;
        case "pick-folder": {
          const picked = await vscode.window.showOpenDialog({
            canSelectFiles: false,
            canSelectFolders: true,
            canSelectMany: false,
          });
          this.post({ kind: "folder-picked", path: picked?.[0]?.fsPath ?? null });
          return;
        }
      }
    } catch (e) {
      this.post({ kind: "toast", text: String(e) });
    }
  }

  private startTask(beatId: number, prompt: string, images: string[]): void {
    this.running.add(beatId);
    native
      .runTask(beatId, prompt, images, (ev) => {
        const { beatId: _tag, ...event } = ev;
        this.post({ kind: "task-event", beatId: ev.beatId, ev: event });
      })
      .then((result) => {
        this.post({ kind: "task-result", beatId, result });
        this.post({ kind: "beats", beats: native.listBeats() });
        this.post({ kind: "usage-totals", beatId, totals: native.usageTotals(beatId) });
      })
      .catch((e: unknown) => {
        this.post({ kind: "task-error", beatId, error: String(e) });
      })
      .finally(() => {
        this.running.delete(beatId);
      });
  }

  private startWorkflow(
    beatId: number,
    workflowId: string,
    prompt: string,
    images: string[],
  ): void {
    this.running.add(beatId);
    native
      .runWorkflowTask(beatId, workflowId, prompt, images, (ev) => {
        const { beatId: _tag, ...event } = ev;
        this.post({ kind: "task-event", beatId: ev.beatId, ev: event });
      })
      .then((result) => {
        this.post({ kind: "task-result", beatId, result });
        this.post({ kind: "beats", beats: native.listBeats() });
        this.post({ kind: "usage-totals", beatId, totals: native.usageTotals(beatId) });
      })
      .catch((e: unknown) => {
        this.post({ kind: "task-error", beatId, error: String(e) });
      })
      .finally(() => {
        this.running.delete(beatId);
      });
  }

  public dispose(): void {
    PulsePanel.current = undefined;
    this.panel.dispose();
    for (const d of this.disposables) d.dispose();
    this.disposables = [];
  }
}

function getNonce(): string {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  let out = "";
  for (let i = 0; i < 32; i++) out += chars[Math.floor(Math.random() * chars.length)];
  return out;
}
