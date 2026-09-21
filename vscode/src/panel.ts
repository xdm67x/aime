import * as vscode from "vscode";
import { PulseBridge } from "./bridge";

export class PulsePanel {
  public static current: PulsePanel | undefined;

  private static readonly viewType = "pulse.harness";
  private readonly panel: vscode.WebviewPanel;
  private readonly bridge: PulseBridge;
  private disposables: vscode.Disposable[] = [];

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
    this.bridge = new PulseBridge(panel.webview, extensionUri, (msg) => {
      void panel.webview.postMessage(msg);
    });
    panel.onDidDispose(() => this.dispose(), undefined, this.disposables);
    PulsePanel.current = this;
  }

  public dispose(): void {
    PulsePanel.current = undefined;
    this.bridge.dispose();
    this.panel.dispose();
    for (const d of this.disposables) d.dispose();
    this.disposables = [];
  }
}
