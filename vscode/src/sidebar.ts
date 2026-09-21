import * as vscode from "vscode";
import { PulseBridge } from "./bridge";

/**
 * Pulse in the activity-bar sidebar: a `WebviewView` reusing the same React
 * UI and message bridge as the editor panel opened by `pulse.open`.
 */
export class PulseSidebarProvider implements vscode.WebviewViewProvider {
  public static readonly viewType = "pulse.harness.sidebar";

  private view?: vscode.WebviewView;
  private bridge?: PulseBridge;

  public constructor(private readonly extensionUri: vscode.Uri) {}

  public resolveWebviewView(view: vscode.WebviewView): void {
    this.view = view;
    view.webview.options = {
      enableScripts: true,
      localResourceRoots: [vscode.Uri.joinPath(this.extensionUri, "webview-dist")],
    };
    this.bridge = new PulseBridge(view.webview, this.extensionUri, (msg) => {
      void view.webview.postMessage(msg);
    });
  }

  public dispose(): void {
    this.bridge?.dispose();
    this.bridge = undefined;
    this.view = undefined;
  }
}
