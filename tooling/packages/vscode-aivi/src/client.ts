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

  // The aivi binary links against GTK4/libwayland even for headless
  // subcommands like `lsp`. Prevent display-server interaction by
  // clearing Wayland/X11 env vars — otherwise the child process can
  // corrupt the compositor's keyboard state, causing "stuck key"
  // auto-repeat in the editor.
  const lspEnv: Record<string, string> = { ...process.env } as Record<
    string,
    string
  >;
  delete lspEnv["WAYLAND_DISPLAY"];
  delete lspEnv["WAYLAND_SOCKET"];
  delete lspEnv["DISPLAY"];
  delete lspEnv["GDK_BACKEND"];

  const serverOptions: ServerOptions = {
    run: {
      command: config.compilerPath,
      args: ["lsp", ...config.compilerArgs],
      transport: TransportKind.stdio,
      options: { env: lspEnv },
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
      options: { env: lspEnv },
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
      inlayHintsMaxLength: config.inlayHintsMaxLength,
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
