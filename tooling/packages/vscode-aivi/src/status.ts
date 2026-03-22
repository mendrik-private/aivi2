import * as vscode from "vscode";

export type ServerStatus = "starting" | "running" | "error" | "crashed";

export class StatusBarItem {
  private item: vscode.StatusBarItem;
  private errorCount = 0;

  constructor() {
    this.item = vscode.window.createStatusBarItem(
      vscode.StatusBarAlignment.Left,
      10
    );
    this.item.command = "aivi.showOutputChannel";
    this.setStatus("starting");
    this.item.show();
  }

  setStatus(status: ServerStatus, errorCount?: number): void {
    this.errorCount = errorCount ?? this.errorCount;
    switch (status) {
      case "starting":
        this.item.text = "$(loading~spin) AIVI";
        this.item.tooltip = "AIVI language server starting...";
        break;
      case "running":
        if (this.errorCount > 0) {
          this.item.text = `$(error) AIVI (${this.errorCount} error${this.errorCount === 1 ? "" : "s"})`;
          this.item.tooltip = `AIVI: ${this.errorCount} error(s) in workspace`;
        } else {
          this.item.text = "$(check) AIVI";
          this.item.tooltip = "AIVI language server running";
        }
        break;
      case "error":
        this.item.text = "$(warning) AIVI";
        this.item.tooltip = "AIVI language server encountered an error";
        break;
      case "crashed":
        this.item.text = "$(alert) AIVI \u2014 click to restart";
        this.item.tooltip = "AIVI language server crashed. Click to restart.";
        this.item.command = "aivi.restartServer";
        break;
    }
  }

  dispose(): void {
    this.item.dispose();
  }
}
