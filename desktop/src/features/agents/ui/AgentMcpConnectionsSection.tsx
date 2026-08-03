import * as React from "react";
import { Plus, Server, Trash2 } from "lucide-react";

import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { cn } from "@/shared/lib/cn";
import {
  PERSONA_FIELD_CONTROL_CLASS,
  PERSONA_FIELD_SHELL_CLASS,
} from "./agentConfigOptions";
import {
  newAgentMcpConnection,
  parseAgentMcpConnections,
  validateAgentMcpConnections,
  writeAgentMcpConnections,
  type AgentMcpConnection,
  type AgentMcpTransport,
} from "./agentMcpConnections";

export function AgentMcpConnectionsSection({
  disabled,
  envVars,
  onChange,
  onValidityChange,
}: {
  disabled: boolean;
  envVars: Record<string, string>;
  onChange: (next: Record<string, string>) => void;
  onValidityChange: (valid: boolean) => void;
}) {
  const [source] = React.useState(() => parseAgentMcpConnections(envVars));
  const [connections, setConnections] = React.useState<AgentMcpConnection[]>(
    source.connections,
  );

  const validationError =
    source.error ?? validateAgentMcpConnections(connections);
  React.useEffect(() => {
    onValidityChange(validationError === null);
  }, [onValidityChange, validationError]);

  function commit(next: AgentMcpConnection[]) {
    setConnections(next);
    onChange(
      writeAgentMcpConnections({
        baseEnvVars: envVars,
        connections: next,
        previous: source,
      }),
    );
  }

  function patchConnection(id: string, patch: Partial<AgentMcpConnection>) {
    commit(
      connections.map((connection) =>
        connection.id === id ? { ...connection, ...patch } : connection,
      ),
    );
  }

  return (
    <section className="space-y-3 rounded-2xl border border-border/70 bg-muted/20 p-4">
      <div className="flex items-start justify-between gap-3">
        <div className="flex min-w-0 gap-3">
          <span className="flex h-9 w-9 shrink-0 items-center justify-center rounded-full bg-muted">
            <Server className="h-4 w-4 text-muted-foreground" />
          </span>
          <div>
            <h3 className="text-sm font-medium text-foreground">
              MCP connections
            </h3>
            <p className="mt-0.5 text-xs text-muted-foreground">
              Connect remote MCP servers to this agent only. Credentials are
              stored with this agent and allowed tools are enforced by Buzz
              Agent.
            </p>
          </div>
        </div>
        <Button
          disabled={
            disabled || source.error !== null || connections.length >= 15
          }
          onClick={() => commit([...connections, newAgentMcpConnection()])}
          size="sm"
          type="button"
          variant="outline"
        >
          <Plus className="mr-1.5 h-3.5 w-3.5" />
          Add
        </Button>
      </div>

      {connections.length === 0 && !source.error ? (
        <p className="rounded-xl border border-dashed border-border px-3 py-4 text-center text-xs text-muted-foreground">
          No custom MCP connections yet.
        </p>
      ) : null}

      <div className="space-y-3">
        {connections.map((connection, index) => (
          <div
            className="space-y-3 rounded-xl border border-border/70 bg-background/70 p-3"
            key={connection.id}
          >
            <div className="flex items-center justify-between gap-2">
              <p className="text-xs font-medium text-foreground">
                Connection {index + 1}
              </p>
              <Button
                aria-label={`Remove MCP connection ${index + 1}`}
                disabled={disabled}
                onClick={() =>
                  commit(
                    connections.filter((item) => item.id !== connection.id),
                  )
                }
                size="icon"
                type="button"
                variant="ghost"
              >
                <Trash2 className="h-4 w-4 text-muted-foreground" />
              </Button>
            </div>

            <div className="grid gap-3 sm:grid-cols-2">
              <Field label="Name">
                <Input
                  aria-label="MCP name"
                  autoCorrect="off"
                  className={PERSONA_FIELD_CONTROL_CLASS}
                  disabled={disabled}
                  onChange={(event) =>
                    patchConnection(connection.id, { name: event.target.value })
                  }
                  placeholder="crm"
                  value={connection.name}
                />
              </Field>
              <Field label="Transport">
                <select
                  aria-label="MCP transport"
                  className={cn(
                    "h-10 w-full px-3 text-sm",
                    PERSONA_FIELD_SHELL_CLASS,
                    PERSONA_FIELD_CONTROL_CLASS,
                  )}
                  disabled={disabled}
                  onChange={(event) =>
                    patchConnection(connection.id, {
                      transport: event.target.value as AgentMcpTransport,
                    })
                  }
                  value={connection.transport}
                >
                  <option value="http-first">Auto (HTTP first)</option>
                  <option value="http-only">Streamable HTTP only</option>
                  <option value="sse-first">Auto (SSE first)</option>
                  <option value="sse-only">SSE only</option>
                </select>
              </Field>
            </div>

            <Field label="Server URL">
              <Input
                aria-label="MCP server URL"
                autoCapitalize="none"
                autoCorrect="off"
                className={PERSONA_FIELD_CONTROL_CLASS}
                disabled={disabled}
                onChange={(event) =>
                  patchConnection(connection.id, { url: event.target.value })
                }
                placeholder="https://mcp.example.com/mcp"
                type="url"
                value={connection.url}
              />
            </Field>

            <div className="grid gap-3 sm:grid-cols-2">
              <Field label="Authentication">
                <select
                  aria-label="MCP authentication"
                  className={cn(
                    "h-10 w-full px-3 text-sm",
                    PERSONA_FIELD_SHELL_CLASS,
                    PERSONA_FIELD_CONTROL_CLASS,
                  )}
                  disabled={disabled}
                  onChange={(event) =>
                    patchConnection(connection.id, {
                      authType: event.target.value as "none" | "bearer",
                    })
                  }
                  value={connection.authType}
                >
                  <option value="none">No authentication</option>
                  <option value="bearer">Bearer token</option>
                </select>
              </Field>
              {connection.authType === "bearer" ? (
                <Field label="Bearer token">
                  <Input
                    aria-label="MCP bearer token"
                    autoComplete="off"
                    className={PERSONA_FIELD_CONTROL_CLASS}
                    disabled={disabled}
                    onChange={(event) =>
                      patchConnection(connection.id, {
                        bearerToken: event.target.value,
                      })
                    }
                    placeholder="Paste token…"
                    type="password"
                    value={connection.bearerToken}
                  />
                </Field>
              ) : null}
            </div>

            <Field label="Allowed tools (optional)">
              <Input
                aria-label="MCP allowed tools"
                autoCapitalize="none"
                autoCorrect="off"
                className={PERSONA_FIELD_CONTROL_CLASS}
                disabled={disabled}
                onChange={(event) =>
                  patchConnection(connection.id, {
                    allowedTools: event.target.value,
                  })
                }
                placeholder="search_contacts, create_note"
                value={connection.allowedTools}
              />
              <p className="mt-1 text-2xs text-muted-foreground">
                Leave empty to expose all tools. Use exact tool names separated
                by commas.
              </p>
            </Field>
          </div>
        ))}
      </div>

      {source.unmanaged.length > 0 ? (
        <p className="text-xs text-muted-foreground">
          {source.unmanaged.length} advanced stdio MCP connection
          {source.unmanaged.length === 1 ? " is" : "s are"} preserved and can be
          managed through the existing raw-profile workflow.
        </p>
      ) : null}
      {validationError ? (
        <p className="text-xs text-destructive">{validationError}</p>
      ) : null}
      <p className="text-2xs text-muted-foreground/80">
        OAuth-based cloud connections still require an OAuth callback bridge;
        this screen currently supports no-auth and bearer-token servers.
      </p>
    </section>
  );
}

function Field({
  children,
  label,
}: {
  children: React.ReactNode;
  label: string;
}) {
  return (
    <div className="block space-y-1.5 text-xs font-medium text-foreground">
      <span>{label}</span>
      <div className={cn("rounded-xl", PERSONA_FIELD_SHELL_CLASS)}>
        {children}
      </div>
    </div>
  );
}
