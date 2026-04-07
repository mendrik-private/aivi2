import * as vscode from "vscode";
import type { LanguageClient } from "vscode-languageclient/node";
import { createClient } from "./client";
import { StatusBarItem } from "./status";
import { registerCommands } from "./commands";
import { getConfig } from "./config";

let client: LanguageClient | undefined;
let statusBar: StatusBarItem | undefined;
let outputChannel: vscode.OutputChannel;

const THEME_NAME = "AIVI Dark";
const THEME_PROMPTED_KEY = "aivi.themePrompted";
const LSP_START_TIMEOUT_MS = 15_000;

function log(msg: string): void {
  outputChannel?.appendLine(`[aivi] ${msg}`);
}

async function promptThemeOnFirstInstall(
  context: vscode.ExtensionContext
): Promise<void> {
  if (context.globalState.get<boolean>(THEME_PROMPTED_KEY)) return;
  await context.globalState.update(THEME_PROMPTED_KEY, true);

  const current = vscode.workspace
    .getConfiguration("workbench")
    .get<string>("colorTheme");
  if (current === THEME_NAME) return;

  const choice = await vscode.window.showInformationMessage(
    "Welcome to AIVI! Would you like to switch to the AIVI Dark color theme?",
    "Apply Theme",
    "Not Now"
  );
  if (choice === "Apply Theme") {
    await vscode.workspace
      .getConfiguration("workbench")
      .update("colorTheme", THEME_NAME, vscode.ConfigurationTarget.Global);
  }
}

async function startWithTimeout(lc: LanguageClient): Promise<void> {
  await Promise.race([
    lc.start(),
    new Promise<never>((_, reject) =>
      setTimeout(
        () => reject(new Error("Language server failed to start within timeout")),
        LSP_START_TIMEOUT_MS
      )
    ),
  ]);
}

export async function activate(
  context: vscode.ExtensionContext
): Promise<void> {
  outputChannel = vscode.window.createOutputChannel("AIVI");
  const traceOutputChannel = vscode.window.createOutputChannel("AIVI Trace");

  log("Extension activating");

  void promptThemeOnFirstInstall(context);

  statusBar = new StatusBarItem();
  context.subscriptions.push({ dispose: () => statusBar?.dispose() });

  const config = getConfig();
  log(`Compiler path: ${config.compilerPath}`);
  log(`Compiler args: [${config.compilerArgs.join(", ")}]`);

  const restart = async (): Promise<void> => {
    try {
      if (client) {
        log("Stopping previous language server");
        await client.stop();
        client = undefined;
      }
      statusBar?.setStatus("starting");
      statusBar?.show();
      log("Creating language client");
      client = createClient(context, outputChannel, traceOutputChannel);
      log("Registering state handler");
      client.onDidChangeState((event) => {
        log(`Client state: ${event.oldState} -> ${event.newState}`);
        if (event.newState === 2) {
          statusBar?.setStatus("running");
        } else if (event.newState === 3) {
          statusBar?.setStatus("crashed");
        }
      });
      log("Starting language server...");
      await startWithTimeout(client);
      log("Language server started");
      statusBar?.setStatus("running");
    } catch (err) {
      const msg = err instanceof Error
        ? `${err.message}\n${err.stack ?? ""}`
        : String(err);
      log(`Failed to start language server: ${msg}`);
      statusBar?.setStatus("crashed");
      if (client) {
        try { await client.stop(); } catch { /* ignore */ }
        client = undefined;
      }
    }
  };

  registerCommands(context, () => client, restart, outputChannel);

  context.subscriptions.push(
    vscode.workspace.onWillSaveTextDocument(async (event) => {
      if (event.document.languageId !== "aivi") return;
      const cfg = vscode.workspace.getConfiguration("aivi");
      if (!cfg.get<boolean>("format.onSave")) return;
      event.waitUntil(
        vscode.commands.executeCommand<vscode.TextEdit[]>(
          "vscode.executeFormatDocumentProvider",
          event.document.uri
        )
      );
    })
  );

  context.subscriptions.push(
    vscode.workspace.onDidChangeConfiguration((e) => {
      if (
        e.affectsConfiguration("aivi.compiler.path") ||
        e.affectsConfiguration("aivi.compiler.args") ||
        e.affectsConfiguration("aivi.diagnostics.debounceMs") ||
        e.affectsConfiguration("aivi.inlayHints.enabled") ||
        e.affectsConfiguration("aivi.inlayHints.maxLength") ||
        e.affectsConfiguration("aivi.codeLens.enabled")
      ) {
        void restart();
      }
    })
  );

  // Defer LSP start: don't block activate()
  const hasAiviFile = vscode.workspace.textDocuments.some(
    (d) => d.languageId === "aivi"
  );

  if (hasAiviFile) {
    log("AIVI file already open — starting LSP");
    restart().catch((err) => log(`Unhandled restart error: ${err}`));
  } else {
    log("No AIVI file open — waiting for one");
    const sub = vscode.workspace.onDidOpenTextDocument((doc) => {
      if (doc.languageId !== "aivi") return;
      sub.dispose();
      log("AIVI file opened — starting LSP");
      restart().catch((err) => log(`Unhandled restart error: ${err}`));
    });
    context.subscriptions.push(sub);
  }

  log("Extension activated");
}

export async function deactivate(): Promise<void> {
  if (client) {
    await client.stop();
    client = undefined;
  }
}
