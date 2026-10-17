import * as React from "react";
import type { PersonaCapabilities } from "@/shared/api/personaTypes";
import { Input } from "@/shared/ui/input";
import { Textarea } from "@/shared/ui/textarea";

type McpServerConfig = NonNullable<PersonaCapabilities["mcp_servers"]>[number];

const MCP_KEYS = new Set(["name", "command", "args", "env"]);

/** Accept only the exact stdio shape Rust stores; unknown keys would be dropped. */
function parseMcpServers(text: string): McpServerConfig[] | null {
  if (!text.trim()) return [];
  try {
    const parsed: unknown = JSON.parse(text);
    if (!Array.isArray(parsed)) return null;
    const valid = parsed.every((server) => {
      if (!server || typeof server !== "object" || Array.isArray(server))
        return false;
      const { name, command, args, env } = server as Record<string, unknown>;
      return (
        Object.keys(server).every((key) => MCP_KEYS.has(key)) &&
        typeof name === "string" &&
        typeof command === "string" &&
        (args === undefined ||
          (Array.isArray(args) &&
            args.every((arg) => typeof arg === "string"))) &&
        (env === undefined ||
          (typeof env === "object" &&
            env !== null &&
            !Array.isArray(env) &&
            Object.values(env).every((entry) => typeof entry === "string")))
      );
    });
    return valid ? (parsed as McpServerConfig[]) : null;
  } catch {
    return null;
  }
}

function skillLines(text: string): string[] {
  return text
    .split(/\r?\n/)
    .map((line) => line.trim())
    .filter(Boolean);
}

/** Explicit assignments; native user defaults remain available to the harness. */
export function PersonaCapabilitiesEditor({
  value,
  onChange,
  onValidityChange,
}: {
  value: PersonaCapabilities;
  onChange: (value: PersonaCapabilities) => void;
  onValidityChange: (valid: boolean) => void;
}) {
  const id = React.useId();
  const [mcp, setMcp] = React.useState(() =>
    JSON.stringify(value.mcp_servers ?? [], null, 2),
  );
  // Raw text keeps a just-typed line break; the saved list is derived from it.
  const [skillsText, setSkillsText] = React.useState(() =>
    (value.skills ?? []).join("\n"),
  );
  const [error, setError] = React.useState<string | null>(null);
  return (
    <fieldset className="space-y-3">
      <legend className="text-sm font-medium">
        Skills, tools and lifecycle
      </legend>
      <p className="text-xs text-muted-foreground">
        Assigned skills are added for this agent. Existing harness defaults
        remain available; these assignments do not restrict filesystem access.
      </p>
      <label className="block space-y-1 text-sm" htmlFor={`${id}-cwd`}>
        <span>Working folder</span>
        <Input
          id={`${id}-cwd`}
          value={value.working_directory ?? ""}
          placeholder="Absolute folder path; blank uses the Buzz workspace"
          onChange={(e) =>
            onChange({
              ...value,
              working_directory: e.target.value || undefined,
            })
          }
        />
      </label>
      <label className="block space-y-1 text-sm" htmlFor={`${id}-skills`}>
        <span>Skill folders — one per line</span>
        <Textarea
          id={`${id}-skills`}
          value={skillsText}
          onChange={(e) => {
            setSkillsText(e.target.value);
            onChange({ ...value, skills: skillLines(e.target.value) });
          }}
        />
      </label>
      <p className="text-xs text-muted-foreground">
        Each folder must contain SKILL.md. Relative paths use the working
        folder. Native metadata and supporting scripts are preserved. Skill
        delivery supports Claude Code, Codex and OpenCode.
      </p>
      <label className="block space-y-1 text-sm" htmlFor={`${id}-mcp`}>
        <span>MCP connections — JSON list</span>
        <Textarea
          id={`${id}-mcp`}
          value={mcp}
          aria-invalid={!!error}
          aria-describedby={error ? `${id}-mcp-error` : undefined}
          onChange={(e) => {
            setMcp(e.target.value);
            const parsed = parseMcpServers(e.target.value);
            if (parsed) {
              setError(null);
              onValidityChange(true);
              onChange({ ...value, mcp_servers: parsed });
            } else {
              setError(
                "Enter a JSON list of connections. Each needs a name and command; args is a list of text and env maps names to text. No other fields are supported.",
              );
              onValidityChange(false);
            }
          }}
        />
      </label>
      {error && (
        <p
          id={`${id}-mcp-error`}
          role="alert"
          className="text-sm text-destructive"
        >
          {error}
        </p>
      )}
      <p className="text-xs text-muted-foreground">
        Use an environment reference such as {`\${SERVICE_TOKEN}`} for
        credentials. Saved snapshot files include these settings; snapshots sent
        through Buzz and unlocked cards omit them.
      </p>
      {(["on_start", "on_message", "on_stop"] as const).map((event) => (
        <label
          key={event}
          className="block space-y-1 text-sm"
          htmlFor={`${id}-${event}`}
        >
          <span>
            {
              {
                on_start: "Start script",
                on_message: "Before-message script",
                on_stop: "Stop script",
              }[event]
            }
          </span>
          <Input
            id={`${id}-${event}`}
            value={value.hooks?.[event] ?? ""}
            onChange={(e) => {
              const hooks = {
                ...value.hooks,
                [event]: e.target.value || undefined,
              };
              // An all-empty hook object would still count as an assignment.
              const hasHook = Object.values(hooks).some(Boolean);
              onChange({ ...value, hooks: hasHook ? hooks : undefined });
            }}
          />
        </label>
      ))}
      <p className="text-xs text-muted-foreground">
        Scripts run with your access and a five-second limit. A failed start
        aborts launch; a failed before-message script drops the prompt. Script
        output never replaces the prompt. Stop failures are logged.
      </p>
    </fieldset>
  );
}
