// Desktop-side executor for the web's "desktop command proxy" over the direct (tailnet) connection.
// The web app runs the desktop's own Agents UI; its data commands (list_managed_agents, ...) are forwarded here by name.
// Only the commands in this table can run. Reads return the desktop's raw answer with secrets removed; writes forward only
// validated fields (never commands, args, env var values, prompts or key material). Pure: IO is injected via `invokeRaw`.
import { MgmtError } from "./mgmtError";

export type RawInvoke = (cmd: string, args?: Record<string, unknown>) => Promise<unknown>;
export type CmdDeps = { invokeRaw: RawInvoke; ownerPubkey?: string };

const HEX64 = /^[0-9a-f]{64}$/;
// biome-ignore lint/suspicious/noControlCharactersInRegex: rejecting control characters is the point
const CONTROL = /[\u0000-\u001f\u007f]/;
const bad = (m: string): never => {
  throw new MgmtError("bad_request", m);
};
const obj = (v: unknown): Record<string, unknown> => (v && typeof v === "object" && !Array.isArray(v) ? (v as Record<string, unknown>) : {});

function str(v: unknown, field: string, max: number): string | undefined {
  if (v === undefined || v === null || v === "") return undefined;
  if (typeof v !== "string") return bad(`${field} must be text.`);
  const t = v.trim();
  if (t.length > max) return bad(`${field} is too long.`);
  if (CONTROL.test(t)) return bad(`${field} has invalid characters.`);
  return t || undefined;
}
function pubkey(v: unknown): string {
  const p = typeof v === "string" ? v.toLowerCase() : "";
  return HEX64.test(p) ? p : bad("Choose one of your agents.");
}

/** Per-agent opt-in for interactive permission prompts (read by buzz-acp at spawn). */
export const PERMISSION_MODE_ENV = "BUZZ_ACP_PERMISSION_MODE";
/** env var values never leave the desktop: keys stay (the UI shows which are set), values are blanked. */
export function blankEnv(e: unknown): Record<string, string> {
  return Object.fromEntries(Object.keys(obj(e)).map((k) => [k, ""]));
}
export function sanitizeAgent(a: unknown): unknown {
  const r = { ...obj(a) };
  if ("env_vars" in r) {
    r.permission_ask = String(obj(r.env_vars)[PERMISSION_MODE_ENV] ?? "").trim() === "ask";
    r.env_vars = blankEnv(r.env_vars);
  }
  if ("private_key_nsec" in r) r.private_key_nsec = "";
  return r;
}
export function sanitizePersona(p: unknown): unknown {
  const r = { ...obj(p) };
  if ("env_vars" in r) r.env_vars = blankEnv(r.env_vars);
  return r;
}
function sanitizeRuntime(e: unknown): unknown {
  const r = { ...obj(e) };
  if ("definition_env" in r) r.definition_env = blankEnv(r.definition_env);
  return r;
}
export function sanitizeGlobal(c: unknown): unknown {
  const r = { ...obj(c) };
  r.env_vars = blankEnv(r.env_vars);
  return r;
}
const list = (v: unknown, f: (x: unknown) => unknown, max = 500) => (Array.isArray(v) ? v.slice(0, max).map(f) : []);

export const RESPOND_MODES = ["owner-only", "anyone", "allowlist"] as const;
function respondFields(src: Record<string, unknown>, out: Record<string, unknown>) {
  if (src.respondTo !== undefined) {
    if (typeof src.respondTo !== "string" || !(RESPOND_MODES as readonly string[]).includes(src.respondTo)) bad("Unknown access mode.");
    out.respondTo = src.respondTo;
  }
  if (src.respondToAllowlist !== undefined) {
    const l = src.respondToAllowlist;
    if (!Array.isArray(l) || l.length > 500) return bad("The allow list is too long.");
    out.respondToAllowlist = l.map((x) => (typeof x === "string" && HEX64.test(x.toLowerCase()) ? x.toLowerCase() : bad("The allow list has an invalid key.")));
  }
}

async function runtimeFor(deps: CmdDeps, command: string): Promise<void> {
  const cat = list(await deps.invokeRaw("discover_acp_providers", { force: false }), (x) => obj(x), 200) as Array<Record<string, unknown>>;
  const ok = cat.some((r) => r.command === command && r.availability === "available" && r.source !== "custom");
  if (!ok) throw new MgmtError("runtime_unavailable", "Custom harnesses can't be set remotely. Pick an installed harness, or edit it on the desktop.");
}

export const CMD_READS = [
  "list_managed_agents",
  "list_managed_agent_runtimes",
  "list_personas",
  "discover_acp_providers",
  "agent_access_owner_only",
  "get_global_agent_config",
  "get_presence",
  "get_openclaw_workspace_mcp_status",
  "discover_agent_models",
  "get_agent_models",
  "get_term_settings",
  "get_agent_config_surface",
  "get_managed_agent_log",
  "get_agent_memory",
  "list_permission_prompts",
  "call_openclaw_workspace_mcp_tool",
  "gh_pr_list",
  "get_git_identity",
] as const;
export const CMD_WRITES = [
  "start_managed_agent",
  "stop_managed_agent",
  "create_managed_agent",
  "update_managed_agent",
  "create_persona",
  "update_persona",
  "prepare_term_session_launch",
  "add_team_from_catalog",
  "respond_permission_prompt",
  "refresh_openclaw_workspace_mcp",
] as const;
const PROMPT_MAX = 20_000;
async function runtimeIdFor(deps: CmdDeps, id: string): Promise<void> {
  const cat = list(await deps.invokeRaw("discover_acp_providers", { force: false }), (x) => obj(x), 200) as Array<Record<string, unknown>>;
  if (!cat.some((r) => r.id === id && r.availability === "available" && r.source !== "custom"))
    throw new MgmtError("runtime_unavailable", "Custom harnesses can't be set remotely. Pick an installed harness, or edit it on the desktop.");
}
function behaviorOf(v: unknown): Record<string, unknown> | undefined {
  if (v === undefined || v === null) return undefined;
  const b = obj(v);
  const out: Record<string, unknown> = {};
  respondFields(b, out);
  if (b.parallelism !== undefined) {
    if (!Number.isInteger(b.parallelism) || (b.parallelism as number) < 1 || (b.parallelism as number) > 32) bad("Parallelism must be 1-32.");
    out.parallelism = b.parallelism;
  }
  if (b.sessionPolicy !== undefined) {
    if (b.sessionPolicy !== "channel" && b.sessionPolicy !== "thread") bad("Unknown session policy.");
    out.sessionPolicy = b.sessionPolicy;
  }
  return out;
}
async function personaFields(i: Record<string, unknown>, deps: CmdDeps): Promise<Record<string, unknown>> {
  const out: Record<string, unknown> = { displayName: str(i.displayName, "Name", 64) ?? bad("Name is required.") };
  const prompt = typeof i.systemPrompt === "string" ? i.systemPrompt : "";
  if (prompt.length > PROMPT_MAX) bad("The prompt is too long.");
  out.systemPrompt = prompt;
  if (i.description !== undefined && i.description !== null) out.description = str(i.description, "Description", 280) ?? "";
  for (const [k, max] of [["model", 100], ["provider", 100]] as const) {
    const v = str(i[k], k, max);
    if (v) out[k] = v;
  }
  const rt = str(i.runtime, "Harness", 100);
  if (rt) {
    await runtimeIdFor(deps, rt);
    out.runtime = rt;
  }
  const av = str(i.avatarUrl, "Avatar", 500);
  if (av) {
    if (!/^https?:\/\//i.test(av)) bad("The avatar must be a web link.");
    out.avatarUrl = av;
  }
  const pool = i.namePool;
  if (Array.isArray(pool)) out.namePool = pool.slice(0, 100).map((n) => str(n, "Name pool", 64) ?? bad("Invalid name pool."));
  const beh = behaviorOf(i.behavior);
  if (beh) out.behavior = beh;
  return out;
}
/** Model list fields only (no env, paths or tokens). */
function modelsOut(v: unknown): unknown {
  const r = obj(v);
  const s = (x: unknown, max = 300) => (typeof x === "string" ? x.slice(0, max) : null);
  return {
    agentName: s(r.agentName) ?? "",
    agentVersion: s(r.agentVersion) ?? "",
    models: list(r.models, (m) => {
      const o = obj(m);
      return { id: s(o.id) ?? "", name: s(o.name), description: s(o.description, 1000) };
    }, 500),
    agentDefaultModel: s(r.agentDefaultModel),
    selectedModel: s(r.selectedModel),
    supportsSwitching: r.supportsSwitching === true,
  };
}

const SECRET_NAME = /(token|secret|passw|api[_-]?key|apikey|auth|credential|cookie|bearer|private|session[_-]?key|access[_-]?key)/i;
/** Masks obvious credentials in free text (log lines, config values). */
export function redactSecrets(text: string): string {
  return text
    .replace(/\b([A-Za-z0-9_]*(?:TOKEN|SECRET|PASSWORD|API_KEY|APIKEY|ACCESS_KEY|PRIVATE_KEY)[A-Za-z0-9_]*)(\s*[=:]\s*)("[^"]*"|'[^']*'|\S+)/gi, "$1$2[redacted]")
    .replace(/\b(Bearer|Basic)\s+[A-Za-z0-9._~+\/=-]{8,}/g, "$1 [redacted]")
    .replace(/\b(?:sk|rk|xox[abpr]|ghp|gho|ghs|github_pat|glpat|AKIA|nsec1)[A-Za-z0-9_-]{10,}/g, "[redacted]")
    .replace(/eyJ[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{10,}\.[A-Za-z0-9_-]{5,}/g, "[redacted]");
}
/** Runtime config surface for the read-only web panel: settings and MCP server names only, no secret-looking values. */
function configSurfaceOut(v: unknown): unknown {
  const r = obj(v);
  const field = (f: unknown) => {
    const x = obj(f);
    const secret = SECRET_NAME.test(String(x.key ?? "")) || SECRET_NAME.test(String(x.label ?? ""));
    return { ...x, value: x.value == null ? null : secret ? "[hidden]" : redactSecrets(String(x.value)) };
  };
  const norm = obj(r.normalized);
  const normalized = Object.fromEntries(
    Object.entries(norm).map(([k, nf]) => {
      if (!nf || typeof nf !== "object") return [k, nf ?? null];
      const n = obj(nf);
      return [k, { ...n, ...(typeof n.value === "string" ? { value: redactSecrets(n.value) } : {}) }];
    }),
  );
  return {
    runtimeId: r.runtimeId ?? null,
    runtimeLabel: r.runtimeLabel ?? null,
    isPreSpawn: r.isPreSpawn === true,
    normalized,
    advanced: list(r.advanced, field, 500),
    extensions: list(r.extensions, (e) => {
      const x = obj(e);
      return { name: String(x.name ?? ""), kind: String(x.kind ?? ""), enabled: x.enabled === true };
    }, 500),
    sources: r.sources ?? {},
    ...(r.claudeConfigDirCustom !== undefined ? { claudeConfigDirCustom: r.claudeConfigDirCustom === true } : {}),
    ...(typeof r.effortConfigId === "string" ? { effortConfigId: r.effortConfigId } : {}),
    ...(Array.isArray(r.effortOptions) ? { effortOptions: r.effortOptions } : {}),
  };
}
async function managedPubkey(deps: CmdDeps, raw: unknown): Promise<string> {
  const pk = typeof raw === "string" && HEX64.test(raw.toLowerCase()) ? raw.toLowerCase() : bad("Invalid pubkey.");
  const agents = list(await deps.invokeRaw("list_managed_agents"), (x) => obj(x), 500) as Array<Record<string, unknown>>;
  if (!agents.some((x) => String(x.pubkey).toLowerCase() === pk)) throw new MgmtError("not_found", "That agent isn't on your desktop.");
  return pk;
}
// ---- OpenClaw workspace MCP over Direct (web project pages: repo files, commits, branches, README) ----
/** Workspace tools the web may call. Writes (write_file, edit, delete, ...) never run remotely. */
export const OPENCLAW_TOOLS = ["list_directory", "stat_path", "read_file", "exec"] as const;
/** Read-only git subcommands the project pages run (see features/projects/lib/*Argv). `null` = any args; a list = required 2nd word. */
const GIT_READS: Record<string, readonly string[] | null> = {
  log: null,
  diff: null,
  show: null,
  status: null,
  "rev-list": null,
  "rev-parse": null,
  "for-each-ref": null,
  "ls-files": null,
  "ls-tree": null,
  "merge-base": null,
  branch: ["--list"],
  worktree: ["list"],
  remote: ["get-url"],
};
/** Options that write files or run external programs. */
const GIT_BAD_ARG = /^(--output|--ext-diff|--textconv|--exec|--upload-pack|--receive-pack|--config|-c$|-C$|--git-dir|--work-tree|--namespace|--open-files-in-pager|-O)/;
/** Files whose contents never leave the desktop, even inside the workspace. */
const SECRET_FILE = /(^|\/)(\.env(\..*)?|\.npmrc|\.netrc|\.pypirc|\.git-credentials|credentials(\.json)?|id_(rsa|dsa|ecdsa|ed25519)[^/]*|[^/]*\.(pem|key|p12|pfx|keystore|jks|kdbx))$|(^|\/)\.(ssh|aws|gnupg|docker|kube)(\/|$)/i;
const URL_CREDS = /(\b[a-z][a-z0-9+.-]*:\/\/)[^/\s:@]+(:[^/\s@]*)?@/gi;
/** Text from the desktop's disk: obvious credentials masked, URL user:token@ removed, size capped. */
export function scrubText(t: unknown, max = 2_000_000): string {
  return redactSecrets(typeof t === "string" ? t : "").replace(URL_CREDS, "$1").slice(0, max);
}
function wsPath(v: unknown, field: string): string {
  const p = str(v, field, 4096) ?? bad(`Missing ${field.toLowerCase()}.`);
  if (SECRET_FILE.test(p)) throw new MgmtError("not_found", "That file stays on your desktop.");
  return p;
}
export function checkGitArgv(argv: unknown): string[] {
  if (!Array.isArray(argv) || argv.length < 2 || argv.length > 64) return bad("Only git read commands can run remotely.");
  const a = argv.map((x) => (typeof x === "string" && x.length <= 4096 && !CONTROL.test(x) ? x : bad("Invalid command.")));
  if (a[0] !== "git") bad("Only git read commands can run remotely.");
  const sub = a[1] as string;
  if (!(sub in GIT_READS)) bad("Only git read commands can run remotely.");
  const need = GIT_READS[sub];
  if (need && !need.includes(a[2] ?? "")) bad("Only git read commands can run remotely.");
  if (sub === "branch" && a.slice(3).some((x) => /^(-[dDmMcCfu]|--(delete|move|copy|force|set-upstream|unset-upstream|edit-description|track|no-track|create-reflog))/.test(x)))
    bad("Only git read commands can run remotely.");
  for (const x of a.slice(2)) if (GIT_BAD_ARG.test(x)) bad("That git option can't be used remotely.");
  // git show <ref>:<path> reads a blob: same secret-file rule as read_file.
  if (sub === "show") for (const x of a.slice(2)) if (x.includes(":") && SECRET_FILE.test(x.slice(x.indexOf(":") + 1))) throw new MgmtError("not_found", "That file stays on your desktop.");
  return a;
}
/** Scrub every string in an MCP tool result (content blocks carry JSON or text). */
function scrubResult(v: unknown, depth = 0): unknown {
  if (depth > 12) return null;
  if (typeof v === "string") return scrubText(v);
  if (Array.isArray(v)) return v.slice(0, 20_000).map((x) => scrubResult(x, depth + 1));
  if (v && typeof v === "object") {
    const out: Record<string, unknown> = {};
    for (const [k, x] of Object.entries(v as Record<string, unknown>)) {
      if (/^(token|headers?|authorization|url|endpoint)$/i.test(k)) continue;
      out[k] = scrubResult(x, depth + 1);
    }
    return out;
  }
  return v;
}
/** Drops directory entries for secret files so their names/paths don't list either. */
function dropSecretEntries(v: unknown, depth = 0): unknown {
  if (depth > 12) return v;
  if (Array.isArray(v)) return v.filter((e) => !(e && typeof e === "object" && typeof (e as { path?: unknown }).path === "string" && SECRET_FILE.test((e as { path: string }).path))).map((x) => dropSecretEntries(x, depth + 1));
  if (v && typeof v === "object") return Object.fromEntries(Object.entries(v as Record<string, unknown>).map(([k, x]) => [k, dropSecretEntries(x, depth + 1)]));
  return v;
}
async function openClawTool(a: Record<string, unknown>, inv: RawInvoke): Promise<unknown> {
  const name = a.name;
  if (typeof name !== "string" || !(OPENCLAW_TOOLS as readonly string[]).includes(name)) bad("That workspace tool isn't available remotely.");
  const i = obj(a.arguments);
  let args: Record<string, unknown>;
  switch (name) {
    case "list_directory": {
      const depth = Number.isInteger(i.depth) ? Math.min(Math.max(i.depth as number, 0), 8) : 1;
      args = { path: wsPath(i.path, "Path"), depth, namesOnly: i.namesOnly === true };
      break;
    }
    case "stat_path":
    case "read_file":
      args = { path: wsPath(i.path, "Path") };
      break;
    default:
      args = { argv: checkGitArgv(i.argv), cwd: wsPath(i.cwd, "Directory") };
  }
  const r = obj(await inv("call_openclaw_workspace_mcp_tool", { name, arguments: args }));
  // Entries are JSON inside text blocks; parse, filter and re-serialize so secret file names don't list.
  const result = scrubResult(r.result);
  const filtered =
    name === "list_directory"
      ? (() => {
          const res = obj(result);
          const content = Array.isArray(res.content) ? res.content : null;
          if (!content) return dropSecretEntries(result);
          return {
            ...res,
            content: content.map((c) => {
              const b = obj(c);
              if (b.type !== "text" || typeof b.text !== "string") return c;
              try {
                return { ...b, text: JSON.stringify(dropSecretEntries(JSON.parse(b.text))) };
              } catch {
                return c;
              }
            }),
            ...(res.structuredContent ? { structuredContent: dropSecretEntries(res.structuredContent) } : {}),
          };
        })()
      : result;
  return { isError: r.isError === true, result: filtered };
}

export const CMD_NAMES: readonly string[] = [...CMD_READS, ...CMD_WRITES];

/** The result of a proxied command, or throws MgmtError. `which` tells the caller whether the call mutates agent state. */
export async function handleCmd(name: string, args: unknown, deps: CmdDeps): Promise<unknown> {
  const a = obj(args);
  const inv = deps.invokeRaw;
  switch (name) {
    case "list_managed_agents":
      return list(await inv(name), sanitizeAgent, 200);
    case "list_managed_agent_runtimes":
      return list(await inv(name), (x) => x, 200);
    case "list_personas":
      return list(await inv(name), sanitizePersona, 200);
    case "discover_acp_providers":
      return list(await inv(name, { force: a.force === true }), sanitizeRuntime, 200);
    case "prepare_term_session_launch": {
      // Term-session card Open from the web: the desktop writes the card's prompt file (and, for Claude, its MCP config) exactly
      // as its own Open does. Only paths come back; the shell line is typed into a remote terminal, which keeps its own
      // "Allow remote terminal" switch and signed-duration checks.
      const i = obj(a.input);
      const sid = str(i.sid, "Session", 128) ?? bad("Missing session id.");
      if (!/^[A-Za-z0-9_-]+$/.test(sid)) bad("Invalid session id.");
      const tool = i.tool === "claude" || i.tool === "codex" ? i.tool : bad("Harness must be claude or codex.");
      const prompt = typeof i.prompt === "string" && i.prompt.length > 0 ? i.prompt : bad("Missing prompt.");
      if (prompt.length > 200_000) bad("The prompt is too long.");
      const cwd = i.cwd == null ? null : (str(i.cwd, "Directory", 4096) ?? null);
      const r = obj(await inv(name, { input: { sid, tool, prompt, cwd, openclawWorkspace: i.openclawWorkspace === true } }));
      const path = (v: unknown) => (typeof v === "string" ? v.slice(0, 4096) : null);
      return {
        promptPath: path(r.promptPath) ?? "",
        claudeConfigDir: path(r.claudeConfigDir),
        openclawWired: r.openclawWired === true,
        userSignerDir: path(r.userSignerDir),
        buzzDevMcpWired: r.buzzDevMcpWired === true,
      };
    }
    case "get_agent_config_surface":
      return configSurfaceOut(await inv(name, { pubkey: await managedPubkey(deps, a.pubkey) }));
    case "get_managed_agent_log": {
      const pk = await managedPubkey(deps, a.pubkey);
      const n = Number.isInteger(a.lineCount) ? Math.min(Math.max(a.lineCount as number, 1), 2000) : 500;
      const r = obj(await inv(name, { pubkey: pk, lineCount: n }));
      return { content: redactSecrets(String(r.content ?? "")).slice(-400_000), log_path: typeof r.log_path === "string" ? r.log_path : "" };
    }
    case "get_agent_memory": {
      // The desktop checks you own the agent before decrypting; only managed agents are allowed here too.
      const r = obj(await inv(name, { agentPubkey: await managedPubkey(deps, a.agentPubkey) }));
      const entry = (e: unknown) => {
        const x = obj(e);
        return { slug: String(x.slug ?? ""), body: redactSecrets(String(x.body ?? "")), eventId: String(x.eventId ?? ""), createdAt: Number(x.createdAt ?? 0), outgoingRefs: list(x.outgoingRefs, (y) => String(y), 500) };
      };
      return { core: r.core ? entry(r.core) : null, memories: list(r.memories, entry, 5000), truncated: r.truncated === true, fetchedAt: Number(r.fetchedAt ?? 0) };
    }
    case "get_presence": {
      // The desktop's own presence answer, so an agent's online dot on the web is exactly what the desktop shows.
      const pks = Array.isArray(a.pubkeys) ? a.pubkeys : bad("pubkeys must be a list.");
      if (pks.length > 500) bad("Too many pubkeys.");
      const keys = pks.map((x) => (typeof x === "string" && HEX64.test(x.toLowerCase()) ? x.toLowerCase() : bad("Invalid pubkey.")));
      const r = obj(await inv(name, { pubkeys: keys }));
      return Object.fromEntries(Object.entries(r).filter(([k, v]) => HEX64.test(k.toLowerCase()) && (v === "online" || v === "away" || v === "offline")));
    }
    case "discover_agent_models": {
      // Model lists for a harness on the desktop. The web names a runtime by its command; only a command from the desktop's
      // own catalog is run, with the catalog's args and env and the desktop's own default env (nothing the web sends is
      // executed or injected).
      const i = obj(a.input);
      const want = str(i.agentCommand, "Harness", 500) ?? bad("Choose a harness.");
      const cat = list(await inv("discover_acp_providers", { force: false }), (x) => obj(x), 200) as Array<Record<string, unknown>>;
      const entry = cat.find((r) => r.command === want && r.availability === "available");
      if (!entry) throw new MgmtError("runtime_unavailable", "That harness isn't installed on your desktop.");
      const provider = str(i.provider, "Provider", 100);
      const global = obj(await inv("get_global_agent_config"));
      const env = global.env_vars && typeof global.env_vars === "object" ? (global.env_vars as Record<string, string>) : {};
      return modelsOut(
        await inv(name, {
          input: {
            agentCommand: entry.command,
            agentArgs: Array.isArray(entry.defaultArgs) ? entry.defaultArgs : [],
            ...(provider ? { provider } : {}),
            envVars: env,
            definitionEnv: entry.definitionEnv && typeof entry.definitionEnv === "object" ? entry.definitionEnv : {},
          },
        }),
      );
    }
    case "get_term_settings": {
      // Buzz Term session preferences live in the desktop frontend's store (no secrets): cwd, shell, scrollback, host.
      const { getTermPreferences } = await import("@/features/terminal/termPreferences");
      const p = getTermPreferences();
      return { cwd: p.cwd || null, shell: p.shell || null, scrollback: p.scrollback ?? null, sessionHost: p.sessionHost ?? null, sessionName: p.sessionName || null };
    }
    case "get_agent_models":
      return modelsOut(await inv(name, { pubkey: pubkey(a.pubkey) }));
    case "get_openclaw_workspace_mcp_status": {
      // Status only: no URL, token or headers leave the desktop.
      const r = obj(await inv(name));
      return { connected: r.connected === true, expiresAt: typeof r.expiresAt === "string" ? r.expiresAt : null, connectedViaRelay: r.connectedViaRelay === true };
    }
    case "agent_access_owner_only":
      return (await inv(name)) === true;
    case "call_openclaw_workspace_mcp_tool":
      return openClawTool(a, inv);
    case "gh_pr_list": {
      // OpenClaw checkout PR badges: the desktop's own `gh pr list --json ...` (its gh login never leaves). Fields re-picked.
      const repo = str(a.repo, "Repository", 200) ?? bad("Missing repository.");
      if (!/^[A-Za-z0-9_.-]+\/[A-Za-z0-9_.-]+$/.test(repo)) bad("Invalid repository. Expected owner/name.");
      let rows: unknown = [];
      try {
        rows = JSON.parse(String(await inv(name, { repo })));
      } catch {
        rows = [];
      }
      const s = (v: unknown, max = 500) => (typeof v === "string" ? scrubText(v, max) : null);
      return JSON.stringify(
        list(rows, (x) => {
          const o = obj(x);
          return { number: Number(o.number) || 0, title: s(o.title) ?? "", state: s(o.state, 20) ?? "", isDraft: o.isDraft === true, url: s(o.url, 300) ?? "", mergedAt: s(o.mergedAt, 40), headRefName: s(o.headRefName, 300) ?? "", baseRefName: s(o.baseRefName, 300) ?? "" };
        }, 100),
      );
    }
    case "get_git_identity": {
      // Your git user.name / user.email, for "your commits" highlighting. Nothing else from git config.
      const r = obj(await inv(name));
      const s = (v: unknown) => (typeof v === "string" ? v.slice(0, 200) : null);
      return { name: s(r.name), email: s(r.email) };
    }
    case "refresh_openclaw_workspace_mcp": {
      const r = obj(await inv(name));
      return { connected: r.connected === true, expiresAt: typeof r.expiresAt === "string" ? r.expiresAt : null, connectedViaRelay: r.connectedViaRelay === true };
    }
    case "get_global_agent_config":
      return sanitizeGlobal(await inv(name));
    case "start_managed_agent":
      return sanitizeAgent(await inv(name, { pubkey: pubkey(a.pubkey), expectedRelayUrl: null, expectedSignerPubkey: deps.ownerPubkey ?? null, replayFloorUnix: null }));
    case "create_persona": {
      const fields = await personaFields(obj(a.input), deps);
      // The only env allowed at creation over Direct: the definition's "Ask before using tools" switch.
      if (String(obj(obj(a.input).envVars)[PERMISSION_MODE_ENV] ?? "").trim() === "ask") fields.envVars = { [PERMISSION_MODE_ENV]: "ask" };
      const r = await inv(name, { input: fields });
      return sanitizePersona(r);
    }
    case "update_persona": {
      const i = obj(a.input);
      const id = str(i.id, "Persona", 100) ?? bad("Choose an agent definition.");
      const r = await inv(name, { input: { id, ...(await personaFields(i, deps)) } });
      return sanitizePersona(r);
    }
    case "stop_managed_agent":
      return sanitizeAgent(await inv(name, { pubkey: pubkey(a.pubkey) }));
    case "create_managed_agent": {
      const i = obj(a.input);
      const out: Record<string, unknown> = { name: str(i.name, "Name", 64) ?? bad("Name is required.") };
      for (const [k, max] of [["personaId", 100], ["provider", 100], ["model", 100]] as const) {
        const v = str(i[k], k, max);
        if (v) out[k] = v;
      }
      if (typeof i.systemPrompt === "string" && i.systemPrompt) {
        if (i.systemPrompt.length > PROMPT_MAX) bad("The prompt is too long.");
        out.systemPrompt = i.systemPrompt;
      }
      if (i.spawnAfterCreate === true) out.spawnAfterCreate = true;
      const cmd = str(i.agentCommand, "Harness", 500);
      if (cmd) {
        await runtimeFor(deps, cmd);
        out.agentCommand = cmd;
        out.harnessOverride = i.harnessOverride === true;
      }
      respondFields(i, out);
      const r = obj(await inv(name, { input: out }));
      return { ...r, agent: sanitizeAgent(r.agent), private_key_nsec: "" };
    }
    case "update_managed_agent": {
      const i = obj(a.input);
      const out: Record<string, unknown> = { pubkey: pubkey(i.pubkey) };
      const nm = str(i.name, "Name", 64);
      if (i.name !== undefined) out.name = nm ?? bad("Name is required.");
      for (const [k, max] of [["provider", 100], ["model", 100]] as const)
        if (k in i) out[k] = str(i[k], k, max) ?? null;
      if (typeof i.systemPrompt === "string") {
        if (i.systemPrompt.length > PROMPT_MAX) bad("The prompt is too long.");
        out.systemPrompt = i.systemPrompt;
      }
      const cmd = str(i.agentCommand, "Harness", 500);
      if (cmd) {
        await runtimeFor(deps, cmd);
        out.agentCommand = cmd;
        out.harnessOverride = i.harnessOverride === true;
      }
      respondFields(i, out);
      if (typeof i.permissionAsk === "boolean") {
        // The only env change allowed over Direct: flip the agent's own permission-ask var, keeping its other env intact.
        const raw = list(await deps.invokeRaw("list_managed_agents"), (x) => obj(x), 500) as Array<Record<string, unknown>>;
        const rec = raw.find((x) => String(x.pubkey ?? "").toLowerCase() === out.pubkey);
        if (!rec) bad("Unknown agent.");
        const env = { ...(obj(rec?.env_vars) as Record<string, string>) };
        if (i.permissionAsk) env[PERMISSION_MODE_ENV] = "ask";
        else if (rec?.persona_id) env[PERMISSION_MODE_ENV] = "bypassPermissions"; // override a definition that asks
        else delete env[PERMISSION_MODE_ENV];
        out.envVars = env;
      }
      if (Object.keys(out).length === 1) bad("Nothing to change.");
      const r = obj(await inv(name, { input: out }));
      return { ...r, agent: sanitizeAgent(r.agent) };
    }
    case "list_permission_prompts": {
      // Opt-in prompts (agent env BUZZ_ACP_PERMISSION_MODE=ask). Tool titles/inputs can carry credentials: redact them.
      const pk = await managedPubkey(deps, a.pubkey);
      return list(await inv(name, { pubkey: pk }), (x) => {
        const o = obj(x);
        const t = (v: unknown, max: number) => redactSecrets(typeof v === "string" ? v : "").slice(0, max);
        return {
          id: typeof o.id === "string" ? o.id.slice(0, 64) : "",
          title: t(o.title, 500),
          kind: t(o.kind, 40),
          detail: t(o.detail, 4000),
          createdAt: typeof o.createdAt === "number" ? o.createdAt : 0,
          options: list(o.options, (y) => {
            const p = obj(y);
            return { optionId: String(p.optionId ?? "").slice(0, 200), name: t(p.name, 200), kind: t(p.kind, 40) };
          }, 10),
        };
      }, 50);
    }
    case "respond_permission_prompt": {
      const pk = await managedPubkey(deps, a.pubkey);
      const requestId = str(a.requestId, "Request", 64) ?? bad("Missing request.");
      if (!/^[0-9-]+$/.test(requestId)) bad("Invalid request.");
      const optionId = str(a.optionId, "Option", 200) ?? bad("Missing option.");
      await inv(name, { pubkey: pk, requestId, optionId });
      return {};
    }
    case "add_team_from_catalog": {
      // Same command the desktop's own catalog "Add" runs: it re-fetches and re-verifies the head from the relay, so the web
      // only names which published team; nothing it sends is trusted as team content.
      const i = obj(a.input);
      const hex = (v: unknown, f: string) =>
        typeof v === "string" && HEX64.test(v.toLowerCase()) ? v.toLowerCase() : bad(`Invalid ${f}.`);
      const input = {
        ownerPubkey: hex(i.ownerPubkey, "owner"),
        teamDTag: str(i.teamDTag, "Team", 200) ?? bad("Missing team."),
        eventId: hex(i.eventId, "event id"),
      };
      const r = obj(await inv(name, { input }));
      const t = obj(r.team);
      return {
        alreadyPresent: r.alreadyPresent === true,
        team: {
          id: String(t.id ?? ""),
          name: String(t.name ?? ""),
          description: typeof t.description === "string" ? t.description : null,
          persona_ids: Array.isArray(t.persona_ids) ? t.persona_ids.slice(0, 200).map(String) : [],
          is_builtin: t.is_builtin === true,
          shared: t.shared === true,
        },
      };
    }
    default:
      throw new MgmtError("unknown_op", "That command isn't available remotely.");
  }
}
