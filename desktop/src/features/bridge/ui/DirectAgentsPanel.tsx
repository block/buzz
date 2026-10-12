// Web "My agents" over the direct (tailnet) connection: the full agent list (stopped ones too), create, rename and
// provider/model/runtime editing. Shown only when the desktop has "Allow remote agent management" on and the host is reachable
// on the tailnet; otherwise the page keeps working over the relay (list/start/stop/message only).
import * as React from "react";

import {
  type AgentEdit,
  type DirectAgent,
  type DirectAgents,
  type DirectPersona,
  type DirectRuntime,
  directErrorCopy,
  isDirectDown,
} from "../directAgents";

export function useDirectState(direct: DirectAgents | null) {
  const subscribe = React.useCallback((fn: () => void) => (direct ? direct.subscribe(fn) : () => {}), [direct]);
  React.useSyncExternalStore(subscribe, () => direct?.version ?? 0);
  return { state: direct?.state ?? "idle", live: direct?.live ?? false, expiresAt: direct?.expiresAt ?? 0, error: direct?.lastError ?? null };
}

const box = { border: "1px solid rgba(128,128,128,.3)", borderRadius: 8, padding: "8px 12px" } as const;
const btn = { fontSize: 12, padding: "4px 10px", marginRight: 6 } as const;
const field = { fontSize: 12, width: "100%", boxSizing: "border-box" } as const;

function clock(ms: number) {
  return new Date(ms).toLocaleTimeString([], { hour: "numeric", minute: "2-digit" });
}

/** The connection line: which path is in use and, for a direct one, how long the signature lasts. */
export function PathBadge(props: { direct: DirectAgents | null; canDirect: boolean; managementOn: boolean; onConnect: () => void; busy: boolean; unreachable: boolean }) {
  const { live, expiresAt, error, state } = useDirectState(props.direct);
  const busy = props.busy || state === "connecting";
  return (
    <div data-testid="agents-path-line" style={{ fontSize: 12, margin: "0 0 10px", display: "flex", gap: 8, alignItems: "center", flexWrap: "wrap" }}>
      <span data-testid="agents-path" data-path={live ? "direct" : "relay"} style={{ border: "1px solid rgba(128,128,128,.5)", borderRadius: 4, padding: "0 6px" }}>
        {live ? "Direct" : "Via relay"}
      </span>
      {live && expiresAt > 0 && <span style={{ opacity: 0.7 }}>signed until {clock(expiresAt)}</span>}
      {!live && props.canDirect && props.managementOn && (
        <button type="button" data-testid="direct-connect" style={btn} disabled={busy} onClick={props.onConnect}>
          {busy ? "Waiting for your signer…" : state === "failed" ? "Retry" : "Connect directly"}
        </button>
      )}
      {!live && props.unreachable && props.managementOn && (
        <span data-testid="direct-unreachable" style={{ opacity: 0.7 }}>
          Direct isn't reachable (connect this device to your tailnet for create, rename and edit).
        </span>
      )}
      {error && !live && !busy && (
        <span data-testid="direct-error" style={{ color: "#c0392b" }}>
          {directErrorCopy(error)}
        </span>
      )}
    </div>
  );
}

/** Direct list refresh cadence while the page is visible (the desktop's listener has no push channel yet). */
export const DIRECT_POLL_MS = 3_000;

export function DirectAgentsSection(props: { direct: DirectAgents; /** Changes whenever the relay snapshot does (a state change on the desktop). */ snapshotTs?: number | null }) {
  const { direct, snapshotTs } = props;
  const { live } = useDirectState(direct);
  const [agents, setAgents] = React.useState<DirectAgent[] | null>(null);
  const [runtimes, setRuntimes] = React.useState<DirectRuntime[]>([]);
  const [personas, setPersonas] = React.useState<DirectPersona[]>([]);
  const [error, setError] = React.useState<string | null>(null);
  const [note, setNote] = React.useState<string | null>(null);
  const [creating, setCreating] = React.useState(false);
  const [editing, setEditing] = React.useState<string | null>(null);
  const [stopping, setStopping] = React.useState<string | null>(null);
  const [busy, setBusy] = React.useState<string | null>(null);

  const refresh = React.useCallback(async () => {
    try {
      setAgents(await direct.list());
    } catch (e) {
      if (!isDirectDown(e)) setError(directErrorCopy(e));
    }
  }, [direct]);
  React.useEffect(() => {
    if (!live) return;
    void refresh();
    void direct.runtimes().then(setRuntimes).catch(() => {});
    void direct.personas().then(setPersonas).catch(() => {});
    // Never reconnect (and so never prompt the signer) from the background poll: only refresh while the signature is live.
    const t = window.setInterval(() => {
      if (!document.hidden && direct.live) void refresh();
    }, DIRECT_POLL_MS);
    const onVisible = () => {
      if (!document.hidden) void refresh();
    };
    document.addEventListener("visibilitychange", onVisible);
    // Any change made through this connection (this component or the controls) updates the list at once: optimistic first, then confirmed.
    const off = direct.onMutation((m) => {
      if (m.op === "start" || m.op === "stop")
        setAgents((cur) => cur?.map((a) => (a.pubkey === m.pubkey ? { ...a, status: m.op === "start" ? "running" : "stopped" } : a)) ?? cur);
      if (m.op === "rename")
        setAgents((cur) => cur?.map((a) => (a.pubkey === m.pubkey && m.name ? { ...a, name: m.name } : a)) ?? cur);
      void refresh();
    });
    return () => {
      window.clearInterval(t);
      document.removeEventListener("visibilitychange", onVisible);
      off();
    };
  }, [live, direct, refresh]);
  // The relay snapshot moved (e.g. the desktop started or stopped an agent): re-read the full list now.
  React.useEffect(() => {
    if (live && snapshotTs != null) void refresh();
  }, [snapshotTs, live, refresh]);

  const run = async (what: string, fn: () => Promise<unknown>, done?: string) => {
    setBusy(what);
    setError(null);
    setNote(null);
    try {
      await fn();
      if (done) setNote(done);
      await refresh();
    } catch (e) {
      setError(directErrorCopy(e));
    }
    setBusy(null);
  };
  if (!live) return null;
  const available = runtimes.filter((r) => r.available);
  return (
    <div data-testid="direct-agents" style={{ display: "grid", gap: 8, margin: "0 0 12px" }}>
      <div>
        <button type="button" data-testid="direct-new" style={btn} onClick={() => setCreating((v) => !v)}>
          {creating ? "Cancel" : "New agent"}
        </button>
      </div>
      {creating && (
        <CreateForm
          runtimes={available}
          personas={personas}
          busy={busy === "create"}
          onSubmit={(a) =>
            run("create", async () => {
              const r = await direct.create(a);
              setCreating(false);
              setNote(r.spawnError ? `Created ${r.agent.name}, but it didn't start: ${r.spawnError}` : `Created ${r.agent.name}.`);
            })
          }
        />
      )}
      {note && (
        <p data-testid="direct-note" style={{ fontSize: 12, margin: 0 }}>
          {note}
        </p>
      )}
      {error && (
        <p data-testid="direct-op-error" style={{ fontSize: 12, color: "#c0392b", margin: 0 }}>
          {error}
        </p>
      )}
      {agents && agents.length === 0 && <p style={{ fontSize: 13, opacity: 0.7, margin: 0 }}>No agents on your desktop yet.</p>}
      <ul style={{ listStyle: "none", padding: 0, margin: 0, display: "grid", gap: 8 }}>
        {(agents ?? []).map((a) => (
          <li key={a.pubkey} data-testid="direct-agent-row" data-status={a.status} style={box}>
            <div style={{ display: "flex", justifyContent: "space-between", gap: 8 }}>
              <strong style={{ fontSize: 14 }}>{a.name}</strong>
              <span style={{ fontSize: 12 }}>{a.status === "running" ? "● running" : a.status.replace("_", " ")}</span>
            </div>
            {(a.runtime || a.provider || a.model) && (
              <div style={{ fontSize: 11, opacity: 0.6 }}>{[a.runtime, a.provider, a.model].filter(Boolean).join(" · ")}</div>
            )}
            {a.lastError && a.status !== "running" && <div style={{ fontSize: 11, color: "#c0392b" }}>{a.lastError}</div>}
            <div style={{ marginTop: 6 }}>
              {a.status !== "running" && (
                <button type="button" data-testid="direct-start" style={btn} disabled={!!busy} onClick={() => void run(`start:${a.pubkey}`, () => direct.start(a.pubkey), `Started ${a.name}.`)}>
                  Start
                </button>
              )}
              {a.status === "running" && (
                <button type="button" data-testid="direct-stop" style={btn} disabled={!!busy} onClick={() => setStopping(a.pubkey)}>
                  Stop…
                </button>
              )}
              <button type="button" data-testid="direct-edit" style={btn} onClick={() => setEditing(editing === a.pubkey ? null : a.pubkey)}>
                {editing === a.pubkey ? "Close" : "Edit"}
              </button>
            </div>
            {stopping === a.pubkey && (
              <div data-testid="direct-stop-confirm" role="alertdialog" style={{ border: "1px solid #c0392b", borderRadius: 8, padding: 8, fontSize: 12, marginTop: 6 }}>
                Stop {a.name} on your desktop? Work in progress will be interrupted.
                <div style={{ marginTop: 6 }}>
                  <button
                    type="button"
                    data-testid="direct-stop-yes"
                    style={btn}
                    onClick={() => {
                      setStopping(null);
                      void run(`stop:${a.pubkey}`, () => direct.stop(a.pubkey), `Stopped ${a.name}.`);
                    }}
                  >
                    Confirm
                  </button>
                  <button type="button" style={btn} onClick={() => setStopping(null)}>
                    Cancel
                  </button>
                </div>
              </div>
            )}
            {editing === a.pubkey && (
              <EditForm
                agent={a}
                runtimes={available}
                busy={busy === `edit:${a.pubkey}`}
                onSubmit={(edit) =>
                  void run(
                    `edit:${a.pubkey}`,
                    async () => {
                      let restart = false;
                      const { name, ...rest } = edit;
                      if (name !== undefined && name !== a.name) await direct.rename(a.pubkey, name);
                      if (Object.keys(rest).length > 0) restart = (await direct.update(a.pubkey, rest)).restartNeeded === true;
                      setEditing(null);
                      setNote(restart ? `Saved. Restart ${a.name} for the change to apply.` : "Saved.");
                    },
                  )
                }
              />
            )}
          </li>
        ))}
      </ul>
    </div>
  );
}

function CreateForm(props: {
  runtimes: DirectRuntime[];
  personas: DirectPersona[];
  busy: boolean;
  onSubmit: (a: { name: string; provider?: string; model?: string; personaId?: string; runtimeId?: string; start?: boolean }) => void;
}) {
  const [name, setName] = React.useState("");
  const [provider, setProvider] = React.useState("");
  const [model, setModel] = React.useState("");
  const [persona, setPersona] = React.useState("");
  const [runtime, setRuntime] = React.useState("");
  const [start, setStart] = React.useState(true);
  return (
    <form
      data-testid="direct-create-form"
      style={{ ...box, display: "grid", gap: 6 }}
      onSubmit={(e) => {
        e.preventDefault();
        props.onSubmit({
          name: name.trim(),
          ...(provider.trim() ? { provider: provider.trim() } : {}),
          ...(model.trim() ? { model: model.trim() } : {}),
          ...(persona ? { personaId: persona } : {}),
          ...(runtime ? { runtimeId: runtime } : {}),
          start,
        });
      }}
    >
      <input data-testid="direct-create-name" style={field} placeholder="Name" value={name} maxLength={64} onChange={(e) => setName(e.target.value)} />
      <select data-testid="direct-create-persona" style={field} value={persona} onChange={(e) => setPersona(e.target.value)}>
        <option value="">No persona</option>
        {props.personas.map((p) => (
          <option key={p.id} value={p.id}>
            {p.name}
          </option>
        ))}
      </select>
      <select data-testid="direct-create-runtime" style={field} value={runtime} onChange={(e) => setRuntime(e.target.value)}>
        <option value="">Default runtime</option>
        {props.runtimes.map((r) => (
          <option key={r.id} value={r.id}>
            {r.label}
          </option>
        ))}
      </select>
      <input data-testid="direct-create-provider" style={field} placeholder="Provider (optional)" value={provider} onChange={(e) => setProvider(e.target.value)} />
      <input data-testid="direct-create-model" style={field} placeholder="Model (optional)" value={model} onChange={(e) => setModel(e.target.value)} />
      <label style={{ fontSize: 12 }}>
        <input type="checkbox" checked={start} onChange={(e) => setStart(e.target.checked)} /> Start after creating
      </label>
      <div>
        <button type="submit" data-testid="direct-create-submit" style={btn} disabled={props.busy || !name.trim()}>
          {props.busy ? "Creating…" : "Create"}
        </button>
      </div>
    </form>
  );
}

function EditForm(props: { agent: DirectAgent; runtimes: DirectRuntime[]; busy: boolean; onSubmit: (e: AgentEdit) => void }) {
  const { agent } = props;
  const [name, setName] = React.useState(agent.name);
  const [provider, setProvider] = React.useState(agent.provider ?? "");
  const [model, setModel] = React.useState(agent.model ?? "");
  const [runtime, setRuntime] = React.useState("");
  return (
    <form
      data-testid="direct-edit-form"
      style={{ display: "grid", gap: 6, marginTop: 8 }}
      onSubmit={(e) => {
        e.preventDefault();
        const edit: AgentEdit = {};
        if (name.trim() && name.trim() !== agent.name) edit.name = name.trim();
        if (provider.trim() !== (agent.provider ?? "")) edit.provider = provider.trim() || null;
        if (model.trim() !== (agent.model ?? "")) edit.model = model.trim() || null;
        if (runtime) edit.runtimeId = runtime;
        if (Object.keys(edit).length > 0) props.onSubmit(edit);
      }}
    >
      <input data-testid="direct-edit-name" style={field} value={name} maxLength={64} onChange={(e) => setName(e.target.value)} />
      <select data-testid="direct-edit-runtime" style={field} value={runtime} onChange={(e) => setRuntime(e.target.value)}>
        <option value="">Keep runtime{agent.runtime ? ` (${agent.runtime})` : ""}</option>
        {props.runtimes.map((r) => (
          <option key={r.id} value={r.id}>
            {r.label}
          </option>
        ))}
      </select>
      <input data-testid="direct-edit-provider" style={field} placeholder="Provider" value={provider} onChange={(e) => setProvider(e.target.value)} />
      <input data-testid="direct-edit-model" style={field} placeholder="Model" value={model} onChange={(e) => setModel(e.target.value)} />
      <div>
        <button type="submit" data-testid="direct-edit-save" style={btn} disabled={props.busy}>
          {props.busy ? "Saving…" : "Save"}
        </button>
      </div>
    </form>
  );
}
