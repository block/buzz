import * as React from "react";

import {
  type AgentClaudeAccount,
  getAgentClaudeAccount,
} from "@/shared/api/tauri";

/**
 * Read-only row showing which Claude account the app's local agents
 * authenticate as. Backed by the `get_agent_claude_account` command, which
 * inspects the Claude Code config directory the agent runtime inherits
 * (`CLAUDE_CONFIG_DIR`, else `$HOME`). Never reads or displays any token.
 */
export function ClaudeAgentAccountRow() {
  const [account, setAccount] = React.useState<AgentClaudeAccount | null>(null);
  const [failed, setFailed] = React.useState(false);

  React.useEffect(() => {
    let cancelled = false;
    getAgentClaudeAccount()
      .then((value) => {
        if (!cancelled) setAccount(value);
      })
      .catch(() => {
        if (!cancelled) setFailed(true);
      });
    return () => {
      cancelled = true;
    };
  }, []);

  let primary = "Checking…";
  if (failed) {
    primary = "Unavailable";
  } else if (account?.source === "oauth") {
    primary = account.email ?? "Signed in";
  } else if (account?.source === "apiKey") {
    primary = "API key (ANTHROPIC_API_KEY)";
  } else if (account?.source === "none") {
    primary = "Not signed in";
  }

  return (
    <div
      className="flex flex-col gap-1 rounded-lg border border-border/70 bg-muted/20 px-3 py-2.5"
      data-testid="settings-claude-agent-account"
    >
      <div className="flex flex-wrap items-center gap-2 text-sm">
        <span className="font-medium text-muted-foreground">
          Claude account
        </span>
        <span className="font-medium text-foreground">{primary}</span>
      </div>
      {account?.source === "oauth" && account.org ? (
        <span className="text-xs text-muted-foreground">{account.org}</span>
      ) : null}
      {account ? (
        <span className="font-mono text-2xs text-muted-foreground">
          {account.configDir}
        </span>
      ) : null}
    </div>
  );
}
