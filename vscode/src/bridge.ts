import * as vscode from "vscode";
import type { HostToWebview, WebviewToHost } from "./protocol";
import * as native from "./native";
import { buildWebviewHtml } from "./webview-html";

/**
 * Shared message-bridge between the extension host and the Pulse webview UI.
 * Backs both the editor panel (`pulse.open`) and the activity-bar sidebar
 * view; each surface attaches its own `post` transport.
 */
export class PulseBridge {
  private running = new Set<number>();
  private disposables: vscode.Disposable[] = [];
  private readonly post: (msg: HostToWebview) => void;

  public constructor(
    webview: vscode.Webview,
    extensionUri: vscode.Uri,
    post: (msg: HostToWebview) => void,
  ) {
    this.post = post;
    this.attach(webview, extensionUri);
  }

  private attach(webview: vscode.Webview, extensionUri: vscode.Uri): void {
    const dist = vscode.Uri.joinPath(extensionUri, "webview-dist");
    webview.html = buildWebviewHtml({
      cspSource: webview.cspSource,
      scriptUri: String(webview.asWebviewUri(vscode.Uri.joinPath(dist, "index.js"))),
      styleUri: String(webview.asWebviewUri(vscode.Uri.joinPath(dist, "index.css"))),
      nonce: getNonce(),
    });
    webview.onDidReceiveMessage(
      (m: WebviewToHost) => void this.onMessage(m),
      undefined,
      this.disposables,
    );
  }

  public dispose(): void {
    for (const d of this.disposables) d.dispose();
    this.disposables = [];
  }

  private send(msg: HostToWebview): void {
    this.post(msg);
  }

  private async onMessage(msg: WebviewToHost): Promise<void> {
    try {
      switch (msg.kind) {
        case "ready":
          native.startModelsRefresh();
          this.send({ kind: "beats", beats: native.listBeats() });
          this.send({ kind: "projects", projects: native.listProjects() });
          this.send({ kind: "model-config", config: native.getModelConfig() });
          try {
            this.send({ kind: "models", models: await native.listModels() });
          } catch (e) {
            this.send({ kind: "toast", text: `Models unavailable: ${e}` });
          }
          return;
        case "run-task":
          this.startTask(msg.beatId, msg.prompt, msg.images);
          return;
        case "cancel-task":
          native.cancelCurrent(msg.beatId);
          return;
        case "list-beats":
          this.send({ kind: "beats", beats: native.listBeats() });
          return;
        case "create-beat": {
          const beat = native.createBeat(msg.name, msg.description, msg.projectId);
          this.send({ kind: "beats", beats: native.listBeats() });
          this.send({ kind: "toast", text: `Beat "${beat.name}" created` });
          return;
        }
        case "archive-beat":
          native.setBeatArchived(msg.beatId, msg.archived);
          this.send({ kind: "beats", beats: native.listBeats() });
          return;
        case "delete-beat":
          this.send({ kind: "toast", text: native.deleteBeat(msg.beatId) });
          this.send({ kind: "beats", beats: native.listBeats() });
          return;
        case "get-beat-messages":
          this.send({
            kind: "beat-messages",
            beatId: msg.beatId,
            messages: native.getBeatMessages(msg.beatId),
          });
          return;
        case "list-projects":
          this.send({ kind: "projects", projects: native.listProjects() });
          return;
        case "add-project":
          native.addProject(msg.path);
          this.send({ kind: "projects", projects: native.listProjects() });
          return;
        case "remove-project":
          native.removeProject(msg.projectId);
          this.send({ kind: "projects", projects: native.listProjects() });
          return;
        case "list-models":
          this.send({ kind: "models", models: await native.listModels() });
          return;
        case "get-model-config":
          this.send({ kind: "model-config", config: native.getModelConfig() });
          return;
        case "save-model-config":
          native.saveModelConfig(msg.config);
          this.send({ kind: "toast", text: "Model slots saved" });
          return;
        case "save-api-key":
          native.saveApiKey(msg.provider, msg.key);
          this.send({ kind: "toast", text: "API key saved" });
          return;
        case "get-api-key":
          this.send({
            kind: "api-key",
            provider: msg.provider,
            key: native.getApiKey(msg.provider),
          });
          return;
        case "beat-usage-totals":
          this.send({
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
          this.send({ kind: "folder-picked", path: picked?.[0]?.fsPath ?? null });
          return;
        }
      }
    } catch (e) {
      this.send({ kind: "toast", text: String(e) });
    }
  }

  private startTask(beatId: number, prompt: string, images: string[]): void {
    this.running.add(beatId);
    native
      .runTask(beatId, prompt, images, (ev) => {
        const { beatId: _tag, ...event } = ev;
        this.send({ kind: "task-event", beatId: ev.beatId, ev: event });
      })
      .then((result) => {
        this.send({ kind: "task-result", beatId, result });
        this.send({ kind: "beats", beats: native.listBeats() });
        this.send({ kind: "usage-totals", beatId, totals: native.usageTotals(beatId) });
      })
      .catch((e: unknown) => {
        this.send({ kind: "task-error", beatId, error: String(e) });
      })
      .finally(() => {
        this.running.delete(beatId);
      });
  }
}

function getNonce(): string {
  const chars = "ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789";
  let out = "";
  for (let i = 0; i < 32; i++) out += chars[Math.floor(Math.random() * chars.length)];
  return out;
}
