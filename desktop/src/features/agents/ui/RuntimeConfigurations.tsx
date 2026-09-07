import { useEffect, useRef, useState } from "react";
import { invoke } from "@tauri-apps/api/core";
import { useIdentityQuery } from "@/shared/api/hooks";
import { useCommunities } from "@/features/communities/useCommunities";
import { useAcpRuntimesQuery, useManagedAgentsQuery } from "../hooks";
import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";
import type { DesktopScope } from "../desktopList";
import { Button } from "@/shared/ui/button";

export type ConfigurationRef = { id: string; revision: string };
type Configuration = ConfigurationRef & {
  name: string;
  host: string;
  runtime: string;
  model: string;
  provider: string | null;
  workspace: string | null;
  credentialRefs: Record<string, string>;
};
type ConfigurationSet = { selected: string | null; entries: Configuration[] };
export type ConfigurationView = {
  configurations: ConfigurationSet;
  updatedAt: string;
  host: string;
  running: ConfigurationRef | null;
  catalog: {
    configuration: ConfigurationRef | null;
    name: string;
    runtime: string;
    model: string | null;
    eligible: boolean;
  }[];
};

/** Management is local and owner-scoped; lifecycle Stop remains an independent control. */
export function RuntimeConfigurations() {
  const owner = useIdentityQuery().data?.pubkey;
  const { activeCommunity } = useCommunities();
  const community = activeCommunity?.relayUrl
    .trim()
    .replace(/^http/, "ws")
    .replace(/\/+$/, "");
  const agents = useManagedAgentsQuery();
  const runtimes = useAcpRuntimesQuery();
  const [agent, setAgent] = useState("");
  if (!owner || !community) return null;
  const local = agents.data?.filter((a) => a.backend.type === "local") ?? [];
  return (
    <section
      className="space-y-3 rounded-lg border p-4"
      aria-label="Runtime configurations"
    >
      <h2 className="text-base font-semibold">Runtime configurations</h2>
      <p className="text-sm text-muted-foreground">
        Named settings on this Desktop. Keys stay here; edits apply only on the
        next Start.
      </p>
      <select
        aria-label="Configure agent"
        value={agent}
        onChange={(event) => setAgent(event.target.value)}
      >
        <option value="">Choose an agent</option>
        {local.map((a) => (
          <option key={a.pubkey} value={a.pubkey}>
            {a.name}
          </option>
        ))}
      </select>
      {agent && local.some((a) => a.pubkey === agent) && (
        <RuntimeConfigurationEditor
          key={`${owner}:${community}:${agent}`}
          scope={{ owner, community }}
          agent={agent}
          runtimes={
            runtimes.isError || runtimes.isPending ? undefined : runtimes.data
          }
        />
      )}
    </section>
  );
}

/** Exported mounted seam: scope remount retires every asynchronous continuation. */
export function RuntimeConfigurationEditor({
  scope,
  agent,
  runtimes,
}: {
  scope: DesktopScope;
  agent: string;
  runtimes: AcpRuntimeCatalogEntry[] | undefined;
}) {
  const [view, setView] = useState<ConfigurationView | null>(null);
  const [draft, setDraft] = useState<Configuration | null>(null);
  const [error, setError] = useState("");
  const [notice, setNotice] = useState("");
  const [busy, setBusy] = useState(false);
  const generation = useRef(0);
  const args = { ...scope, agent };
  useEffect(() => {
    const token = ++generation.current;
    invoke<ConfigurationView>("get_runtime_configurations", {
      owner: scope.owner,
      community: scope.community,
      agent,
    }).then(
      (next) => {
        if (token === generation.current) setView(next);
      },
      () => {
        if (token === generation.current)
          setError("Configurations could not be loaded. Retry.");
      },
    );
    return () => {
      generation.current++;
    };
  }, [scope.owner, scope.community, agent]);
  async function perform(work: () => Promise<ConfigurationView>, success = "") {
    const token = ++generation.current;
    setBusy(true);
    setError("");
    setNotice("");
    try {
      const next = await work();
      if (token !== generation.current) return;
      setView(next);
      setDraft(null);
      setNotice(success);
    } catch {
      if (token === generation.current)
        setError(
          "Operation failed or settings changed. Reload and retry; no new running configuration was confirmed.",
        );
    } finally {
      if (token === generation.current) setBusy(false);
    }
  }
  const reload = () =>
    invoke<ConfigurationView>("get_runtime_configurations", args);
  const save = (configurations: ConfigurationSet) =>
    perform(
      () =>
        invoke<ConfigurationView>("save_runtime_configurations", {
          ...args,
          expectedUpdatedAt: view?.updatedAt,
          configurations,
        }),
      "Saved for next Start. Any running process is unchanged.",
    );
  const selected = view?.configurations.entries.find(
    (c) => c.id === view.configurations.selected,
  );
  const eligible =
    view?.catalog.some(
      (c) => c.configuration?.id === selected?.id && c.eligible,
    ) ?? false;
  const runtime = runtimes?.find((r) => r.id === draft?.runtime);
  function add() {
    if (!view) return;
    setDraft({
      id: crypto.randomUUID(),
      revision: crypto.randomUUID(),
      name: "",
      host: view.host,
      runtime: "",
      model: "",
      provider: null,
      workspace: null,
      credentialRefs: {},
    });
  }
  return (
    <div className="space-y-3">
      {error && <p role="alert">{error}</p>}
      {notice && <p role="status">{notice}</p>}
      <Button disabled={busy} onClick={() => void perform(reload)}>
        Reload configurations
      </Button>
      {view && (
        <>
          <p className="text-sm">
            Next Start:{" "}
            {selected
              ? `${selected.name} · ${selected.runtime} · ${selected.model}`
              : "Default (inherited settings)"}
          </p>
          <p className="text-sm">
            Running revision:{" "}
            {view.running
              ? `${view.running.id} / ${view.running.revision}`
              : "No named launch reported"}
          </p>
          <label>
            Next runtime configuration
            <select
              aria-label="Next runtime configuration"
              disabled={busy || !!draft}
              value={view.configurations.selected ?? ""}
              onChange={(event) =>
                void save({
                  ...view.configurations,
                  selected: event.target.value || null,
                })
              }
            >
              {view.catalog
                .filter(
                  (c) => c.eligible || c.configuration?.id === selected?.id,
                )
                .map((c) => (
                  <option
                    key={c.configuration?.id ?? "default"}
                    value={c.configuration?.id ?? ""}
                    disabled={!c.eligible}
                  >
                    {c.name} · {c.runtime} · {c.model ?? "Inherited model"}
                    {c.eligible ? "" : " (unavailable)"}
                  </option>
                ))}
            </select>
          </label>
          {!eligible && (
            <p className="text-sm">
              Start unavailable. Check this agent’s local key, runtime, model
              and provider setup, then reload. Existing Stop controls remain
              available.
            </p>
          )}
          <Button
            disabled={busy || !!draft || !eligible}
            onClick={() =>
              void perform(async () => {
                await invoke("start_runtime_configuration", {
                  ...args,
                  configuration: selected
                    ? { id: selected.id, revision: selected.revision }
                    : null,
                });
                return reload();
              }, "Start accepted. Model readiness is reported by the running session.")
            }
          >
            Start configuration
          </Button>
          <Button disabled={busy || !!draft || !runtimes} onClick={add}>
            Add configuration
          </Button>
          <ul>
            {view.configurations.entries.map((c) => (
              <li key={c.id}>
                {c.name} · {c.runtime} · {c.model}
                <Button
                  disabled={busy || !!draft}
                  onClick={() => setDraft({ ...c })}
                >
                  Edit {c.name}
                </Button>
                <Button
                  disabled={busy || !!draft}
                  onClick={() =>
                    void save({
                      selected:
                        view.configurations.selected === c.id
                          ? null
                          : view.configurations.selected,
                      entries: view.configurations.entries.filter(
                        (entry) => entry.id !== c.id,
                      ),
                    })
                  }
                >
                  Delete {c.name}
                </Button>
              </li>
            ))}
          </ul>
          {!runtimes && (
            <p role="status">
              Runtime catalog unavailable or loading. Reload the catalog before
              editing.
            </p>
          )}
          {draft && (
            <fieldset disabled={busy} className="space-y-2">
              <legend>Configuration on this Desktop</legend>
              <label>
                Name{" "}
                <input
                  aria-label="Configuration name"
                  maxLength={120}
                  value={draft.name}
                  onChange={(e) => setDraft({ ...draft, name: e.target.value })}
                />
              </label>
              <label>
                Harness{" "}
                <select
                  aria-label="Configuration harness"
                  value={draft.runtime}
                  onChange={(e) =>
                    setDraft({
                      ...draft,
                      runtime: e.target.value,
                      model: "",
                      provider: null,
                    })
                  }
                >
                  <option value="">Choose a harness</option>
                  {runtimes
                    ?.filter((r) => r.source === "builtin")
                    .map((r) => (
                      <option key={r.id} value={r.id}>
                        {r.label}
                      </option>
                    ))}
                </select>
              </label>
              {runtime?.providerEnvVar && (
                <label>
                  Provider{" "}
                  <input
                    aria-label="Configuration provider"
                    value={draft.provider ?? ""}
                    onChange={(e) =>
                      setDraft({
                        ...draft,
                        provider: e.target.value || null,
                        model: "",
                      })
                    }
                  />
                </label>
              )}
              <label>
                Exact model ID{" "}
                <input
                  aria-label="Configuration model"
                  maxLength={512}
                  value={draft.model}
                  onChange={(e) =>
                    setDraft({ ...draft, model: e.target.value })
                  }
                />
              </label>
              <label>
                Workspace (optional absolute path){" "}
                <input
                  aria-label="Configuration workspace"
                  value={draft.workspace ?? ""}
                  onChange={(e) =>
                    setDraft({ ...draft, workspace: e.target.value || null })
                  }
                />
              </label>
              <p className="text-sm">
                Uses independently provisioned local credentials. No keys are
                copied or entered here. An unavailable model fails instead of
                silently substituting another.
              </p>
              <Button
                disabled={!runtime || !draft.name.trim() || !draft.model.trim()}
                onClick={() =>
                  void save({
                    ...view.configurations,
                    entries: [
                      ...view.configurations.entries.filter(
                        (c) => c.id !== draft.id,
                      ),
                      draft,
                    ],
                  })
                }
              >
                Save configuration
              </Button>
              <Button onClick={() => setDraft(null)}>
                Cancel configuration edit
              </Button>
            </fieldset>
          )}
        </>
      )}
    </div>
  );
}
