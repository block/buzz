const RUNTIME_LABELS: Record<string, string> = {
  goose: "Goose",
  "claude-code": "Claude Code",
  "codex-acp": "Codex",
  aider: "Aider",
};

export function runtimeLabel(command: string): string {
  return RUNTIME_LABELS[command] ?? command;
}
