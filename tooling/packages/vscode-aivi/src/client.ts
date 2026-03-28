import * as vscode from "vscode";
import {
  LanguageClient,
  LanguageClientOptions,
  ServerOptions,
  TransportKind,
} from "vscode-languageclient/node";
import { getConfig } from "./config";

export function createClient(
  context: vscode.ExtensionContext,
  outputChannel: vscode.OutputChannel,
  traceOutputChannel: vscode.OutputChannel
): LanguageClient {
  const config = getConfig();

  const serverOptions: ServerOptions = {
    run: {
      command: config.compilerPath,
      args: ["lsp", ...config.compilerArgs],
      transport: TransportKind.stdio,
    },
    debug: {
      command: config.compilerPath,
      args: [
        "lsp",
        "--log",
        "/tmp/aivi-lsp-debug.log",
        "--log-level",
        "debug",
        ...config.compilerArgs,
      ],
      transport: TransportKind.stdio,
    },
  };

  const clientOptions: LanguageClientOptions = {
    documentSelector: [{ language: "aivi" }],
    synchronize: {
      fileEvents: vscode.workspace.createFileSystemWatcher("**/*.aivi"),
    },
    initializationOptions: {
      diagnosticsDebounceMs: config.diagnosticsDebounceMs,
      inlayHintsEnabled: config.inlayHintsEnabled,
      codeLensEnabled: config.codeLensEnabled,
      completionAutoImport: config.completionAutoImport,
    },
    outputChannel,
    traceOutputChannel,
    markdown: { isTrusted: true, supportHtml: false },
  };

  return new LanguageClient(
    "aivi",
    "AIVI Language Server",
    serverOptions,
    clientOptions
  );
}
