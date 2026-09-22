import * as vscode from "vscode";
import { PulsePanel } from "./panel";
import { PulseSidebarProvider } from "./sidebar";

export function activate(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.commands.registerCommand("pulse.open", () => {
      PulsePanel.createOrShow(context.extensionUri);
    }),
    vscode.window.registerWebviewViewProvider(
      PulseSidebarProvider.viewType,
      new PulseSidebarProvider(context.extensionUri),
    ),
  );
}

export function deactivate(): void {
  /* nothing to clean up */
}
