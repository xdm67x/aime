import * as vscode from "vscode";
import { PulsePanel } from "./panel";

export function activate(context: vscode.ExtensionContext): void {
  context.subscriptions.push(
    vscode.commands.registerCommand("pulse.open", () => {
      PulsePanel.createOrShow(context.extensionUri);
    }),
  );
}

export function deactivate(): void {
  /* nothing to clean up */
}
