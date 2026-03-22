import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";
import { createClient } from "./client";
import { StatusBarItem } from "./status";
import { registerCommands } from "./commands";

let client: LanguageClient | undefined;
let statusBar: StatusBarItem | undefined;

export async function activate(
  context: vscode.ExtensionContext
): Promise<void> {
  const outputChannel = vscode.window.createOutputChannel("AIVI");
  const traceOutputChannel = vscode.window.createOutputChannel("AIVI Trace");

  statusBar = new StatusBarItem();
  context.subscriptions.push({ dispose: () => statusBar?.dispose() });

  const restart = async (): Promise<void> => {
    if (client) {
      await client.stop();
      client = undefined;
    }
    statusBar?.setStatus("starting");
    client = createClient(context, outputChannel, traceOutputChannel);
    client.onDidChangeState((event) => {
      // State 2 = Running, State 1 = Starting, State 3 = Stopped
      if (event.newState === 2) {
        statusBar?.setStatus("running");
      } else if (event.newState === 3) {
        statusBar?.setStatus("crashed");
      }
    });
    try {
      await client.start();
      statusBar?.setStatus("running");
    } catch (err) {
      statusBar?.setStatus("crashed");
      outputChannel.appendLine(`Failed to start AIVI language server: ${err}`);
    }
  };

  registerCommands(context, () => client, restart, outputChannel);

  // Register format on save if configured
  context.subscriptions.push(
    vscode.workspace.onWillSaveTextDocument(async (event) => {
      if (event.document.languageId !== "aivi") return;
      const config = vscode.workspace.getConfiguration("aivi");
      if (!config.get<boolean>("format.onSave")) return;
      event.waitUntil(
        vscode.commands.executeCommand<vscode.TextEdit[]>(
          "vscode.executeFormatDocumentProvider",
          event.document.uri
        )
      );
    })
  );

  // Watch config changes to restart server on compiler path change
  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (
        e.affectsConfiguration("aivi.compiler.path") ||
        e.affectsConfiguration("aivi.compiler.args")
      ) {
        void restart();
      }
    })
  );

  await restart();
}

export async function deactivate(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
}
