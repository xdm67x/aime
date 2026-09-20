import * as vscode from "vscode";
import * as beats from "../harness/beats";
import * as config from "../harness/config";
import { cancelCurrent, runTask } from "../harness/harness";
import * as projects from "../harness/projects";
import { listModels, startModelsRefresh } from "../harness/providers";
import type { HostToWebview, WebviewToHost } from "./protocol";

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
    const indexUri = webview.asWebviewUri(vscode.Uri.joinPath(dist, "index.html"));
    const nonce = getNonce();
    const csp = [
      "default-src 'none'",
      `img-src ${webview.cspSource} data: blob:`,
      `style-src ${webview.cspSource} 'unsafe-inline'`,
      `script-src 'strict-dynamic' 'nonce-${nonce}'`,
      `font-src ${webview.cspSource}`,
      `connect-src ${webview.cspSource}`,
    ].join("; ");
    webview.html = `<!DOCTYPE html>
<html lang="en">
  <head>
    <meta charset="UTF-8" />
    <meta http-equiv="Content-Security-Policy" content="${csp}" />
    <meta name="viewport" content="width=device-width, initial-scale=1.0" />
    <title>Pulse</title>
  </head>
  <body>
    <div id="root"></div>
    <script type="module" src="${indexUri}"></script>
  </body>
</html>`;
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
          startModelsRefresh();
          this.post({ kind: "beats", beats: beats.listBeats() });
          this.post({ kind: "projects", projects: projects.listProjects() });
          this.post({ kind: "model-config", config: config.getModelConfig() });
          try {
            this.post({ kind: "models", models: await listModels() });
          } catch (e) {
            this.post({ kind: "toast", text: `Models unavailable: ${e}` });
          }
          return;
        case "run-task":
          this.startTask(msg.beatId, msg.prompt, msg.images);
          return;
        case "cancel-task":
          cancelCurrent(msg.beatId);
          return;
        case "list-beats":
          this.post({ kind: "beats", beats: beats.listBeats() });
          return;
        case "create-beat": {
          const beat = beats.createBeat(msg.name, msg.description, msg.projectId);
          this.post({ kind: "beats", beats: beats.listBeats() });
          this.post({ kind: "toast", text: `Beat “${beat.name}” created` });
          return;
        }
        case "archive-beat":
          beats.setBeatArchived(msg.beatId, msg.archived);
          this.post({ kind: "beats", beats: beats.listBeats() });
          return;
        case "delete-beat":
          this.post({ kind: "toast", text: beats.deleteBeat(msg.beatId) });
          this.post({ kind: "beats", beats: beats.listBeats() });
          return;
        case "get-beat-messages":
          this.post({
            kind: "beat-messages",
            beatId: msg.beatId,
            messages: beats.getBeatMessages(msg.beatId),
          });
          return;
        case "list-projects":
          this.post({ kind: "projects", projects: projects.listProjects() });
          return;
        case "add-project":
          projects.addProject(msg.path);
          this.post({ kind: "projects", projects: projects.listProjects() });
          return;
        case "remove-project":
          projects.removeProject(msg.projectId);
          this.post({ kind: "projects", projects: projects.listProjects() });
          return;
        case "list-models":
          this.post({ kind: "models", models: await listModels() });
          return;
        case "get-model-config":
          this.post({ kind: "model-config", config: config.getModelConfig() });
          return;
        case "save-model-config":
          config.saveModelConfig(msg.config);
          this.post({ kind: "toast", text: "Model slots saved" });
          return;
        case "save-api-key":
          config.saveApiKey(msg.provider, msg.key);
          this.post({ kind: "toast", text: "API key saved" });
          return;
        case "get-api-key":
          this.post({ kind: "api-key", provider: msg.provider, key: config.apiKey(msg.provider) });
          return;
        case "beat-usage-totals":
          this.post({
            kind: "usage-totals",
            beatId: msg.beatId,
            totals: beats.usageTotals(msg.beatId),
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
    runTask(beatId, prompt, images, (ev) => {
      this.post({ kind: "task-event", beatId: ev.beatId, ev: ev });
    })
      .then((result) => {
        this.post({ kind: "task-result", beatId, result });
        this.post({ kind: "beats", beats: beats.listBeats() });
        this.post({ kind: "usage-totals", beatId, totals: beats.usageTotals(beatId) });
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
