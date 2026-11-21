/**
 * Friendly names for the compiled-in agent runtimes, keyed by every identity
 * a profile can carry for them: the catalog id (persona `runtime`), the
 * adapter command (managed agent `agentCommand`), and the catalog aliases.
 * Labels and keys mirror `KNOWN_ACP_RUNTIMES` in
 * `src-tauri/src/managed_agents/discovery/catalog.rs`.
 */
const RUNTIME_LABELS: Record<string, string> = {
  goose: "Goose",
  claude: "Claude Code",
  "claude-agent-acp": "Claude Code",
  "claude-code-acp": "Claude Code",
  "claude-code": "Claude Code",
  claudecode: "Claude Code",
  codex: "Codex",
  "codex-acp": "Codex",
  "buzz-agent": "Buzz Agent",
  aider: "Aider",
};

/** Basename, lowercased, without a Windows `.exe` suffix. */
function normalizeRuntimeIdentity(command: string): string {
  const normalized = command.trim().replace(/\\/g, "/");
  const basename = normalized.split("/").pop() ?? normalized;
  const lower = basename.toLowerCase();
  return lower.endsWith(".exe") ? lower.slice(0, -".exe".length) : lower;
}

/**
 * Label for a runtime id or harness command shown on profile surfaces.
 * Unknown values (custom harnesses) are returned unchanged.
 */
export function runtimeLabel(command: string): string {
  return RUNTIME_LABELS[normalizeRuntimeIdentity(command)] ?? command;
}
