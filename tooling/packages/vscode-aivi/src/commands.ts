import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";

export function registerCommands(
  context: vscode.ExtensionContext,
  getClient: () => LanguageClient | undefined,
  restart: () => Promise<void>,
  outputChannel: vscode.OutputChannel
): void {
  context.subscriptions.push(
    vscode.commands.registerCommand("aivi.restartServer", async () => {
      await restart();
      outputChannel.show();
    }),

    vscode.commands.registerCommand("aivi.showOutputChannel", () => {
      outputChannel.show();
    }),

    vscode.commands.registerCommand("aivi.formatDocument", async () => {
      const editor = vscode.window.activeTextEditor;
      if (editor?.document.languageId === "aivi") {
        await vscode.commands.executeCommand("editor.action.formatDocument");
      }
    }),

    vscode.commands.registerCommand("aivi.checkFile", async () => {
      const editor = vscode.window.activeTextEditor;
      if (!editor) return;
      const doc = editor.document;
      await doc.save();
      const config = vscode.workspace.getConfiguration("aivi");
      const aiviPath = config.get<string>("compiler.path") ?? "aivi";
      const terminal = vscode.window.createTerminal("AIVI Check");
      terminal.sendText(`${aiviPath} check "${doc.fileName}"`);
      terminal.show();
    }),

    vscode.commands.registerCommand("aivi.openCompilerLog", async () => {
      const logPath = context.storageUri?.fsPath;
      if (logPath) {
        const logFile = vscode.Uri.file(logPath + "/aivi-lsp.log");
        try {
          await vscode.window.showTextDocument(logFile);
        } catch {
          vscode.window.showInformationMessage("No compiler log found.");
        }
      }
    })
  );
}
