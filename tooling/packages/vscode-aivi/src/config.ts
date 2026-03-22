import * as vscode from "vscode";

export interface AiviConfig {
  compilerPath: string;
  compilerArgs: string[];
  compilerTimeout: number;
  diagnosticsDebounceMs: number;
  inlayHintsEnabled: boolean;
  inlayHintsMaxLength: number;
  codeLensEnabled: boolean;
  completionAutoImport: boolean;
  traceServer: "off" | "messages" | "verbose";
  formatOnSave: boolean;
}

export function getConfig(): AiviConfig {
  const cfg = vscode.workspace.getConfiguration("aivi");
  return {
    compilerPath: cfg.get<string>("compiler.path") ?? "aivi",
    compilerArgs: cfg.get<string[]>("compiler.args") ?? [],
    compilerTimeout: cfg.get<number>("compiler.timeout") ?? 5000,
    diagnosticsDebounceMs: cfg.get<number>("diagnostics.debounceMs") ?? 200,
    inlayHintsEnabled: cfg.get<boolean>("inlayHints.enabled") ?? true,
    inlayHintsMaxLength: cfg.get<number>("inlayHints.maxLength") ?? 30,
    codeLensEnabled: cfg.get<boolean>("codeLens.enabled") ?? true,
    completionAutoImport: cfg.get<boolean>("completion.autoImport") ?? true,
    traceServer: cfg.get<"off" | "messages" | "verbose">("trace.server") ?? "off",
    formatOnSave: cfg.get<boolean>("format.onSave") ?? false,
  };
}
