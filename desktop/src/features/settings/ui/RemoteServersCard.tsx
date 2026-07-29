import * as React from "react";
import { AlertTriangle, Server } from "lucide-react";

import {
  useBackendProviderProbesQuery,
  useBackendProvidersQuery,
} from "@/features/agents/hooks";
import { NO_BACKEND_PROVIDER_HINT } from "@/features/agents/lib/backendProviderLabel";
import { cn } from "@/shared/lib/cn";
import { Spinner } from "@/shared/ui/spinner";

import {
  type RemoteServerEntry,
  remoteServerEntries,
  remoteServerProbes,
} from "./remoteServerGalleryLogic";
import { SettingsSectionHeader } from "./SettingsSectionHeader";

/**
 * Settings → Agents → Remote servers.
 *
 * The permanent home of "an agent runs on this computer or on a server you
 * own". Before this, a backend provider was visible only inside the create
 * dialog's run-target dropdown, so a user who had not started creating an
 * agent had no way to learn the remote path exists at all.
 *
 * SCOPE, v1: read-only. No add/edit/remove, and no host list. A provider is a
 * binary the user installs on PATH (`buzz-backend-*` — see
 * docs/remote-agents.md), so "adding" one is an install, not a form; and the
 * host is a per-agent decision that the create dialog owns and pins onto the
 * agent record verbatim at create time. Growing CRUD here would either edit
 * saved configs that deployed agents deliberately do not re-read — which reads
 * as a bug — or duplicate the create flow's ownership of the host. Deployment
 * stays in the create flow; this surface reports what is installed.
 */
export function RemoteServersCard() {
  const providersQuery = useBackendProvidersQuery();
  const providers = React.useMemo(
    () => providersQuery.data ?? [],
    [providersQuery.data],
  );
  const probeResults = useBackendProviderProbesQuery(providers);

  // `useQueries` returns a fresh array every render, so this is derived
  // directly rather than memoized — the projection is a map over a handful of
  // rows, and a memo keyed on an unstable array would never hit anyway.
  const entries = remoteServerEntries(
    providers,
    remoteServerProbes(providers, probeResults),
  );

  return (
    <section
      className="min-w-0 space-y-4"
      data-testid="settings-remote-servers"
    >
      <SettingsSectionHeader
        title="Remote servers"
        description="Buzz agents run on this computer or on your own servers. A backend provider is the binary that puts an agent on one — install it on your PATH, then pick the server when you create an agent."
      />

      {/*
        Text, not a spinner. Discovery is a sub-100ms PATH walk, so the common
        case — no provider installed, first entry into Settings → Agents — would
        mount and unmount a spinner inside a frame or two before swapping to the
        empty state. The Settings siblings state it the same way
        (`ChannelTemplatesSettingsCard`, `DoctorSettingsPanel`); a spinner in
        this directory belongs to a mutation the user started, not to a query.
      */}
      {providersQuery.isLoading ? (
        <div
          aria-live="polite"
          className="rounded-2xl bg-muted/20 px-4 py-4 text-sm text-muted-foreground"
          data-testid="remote-server-loading"
          role="status"
        >
          Looking for backend providers&hellip;
        </div>
      ) : entries.length > 0 ? (
        <div
          className="grid grid-cols-1 gap-3 sm:grid-cols-2"
          data-testid="remote-server-gallery"
        >
          {entries.map((entry) => (
            <RemoteServerCard entry={entry} key={entry.id} />
          ))}
        </div>
      ) : (
        <RemoteServersEmptyState />
      )}

      {providersQuery.error instanceof Error ? (
        <p className="line-clamp-3 wrap-break-word rounded-2xl bg-destructive/10 px-4 py-4 text-sm text-destructive">
          {providersQuery.error.message}
        </p>
      ) : null}
    </section>
  );
}

/**
 * One provider row.
 *
 * Same shape as `PresetCard` in `HarnessManagementCard`: icon, label, mono
 * command line, a status pill in the same emerald/muted vocabulary. "Installed"
 * rather than "Detected" because the pill answers a different question — the
 * binary answers the provider protocol, which is not a claim that any host is
 * reachable (`info` opens no connection).
 */
function RemoteServerCard({ entry }: { entry: RemoteServerEntry }) {
  const isReady = entry.status === "ready";

  return (
    <div
      className={cn(
        "relative flex flex-col gap-3 rounded-2xl border px-4 py-4 text-sm transition-colors",
        isReady
          ? "border-emerald-500/20 bg-emerald-500/5"
          : "border-border/60 bg-muted/20",
      )}
      data-testid={`remote-server-${entry.id}`}
    >
      <div className="flex items-center gap-3">
        <span className="flex h-8 w-8 shrink-0 items-center justify-center rounded-full bg-muted/60">
          <Server className="h-4 w-4 text-muted-foreground" />
        </span>
        <div className="min-w-0 flex-1">
          <p className="font-medium leading-none">{entry.label}</p>
          <p className="mt-1 truncate font-mono text-xs text-muted-foreground">
            {entry.binaryPath}
          </p>
        </div>
        {entry.status === "probing" ? (
          <Spinner className="h-4 w-4 shrink-0 text-muted-foreground" />
        ) : isReady ? (
          <span className="inline-flex shrink-0 items-center rounded-md bg-emerald-500/15 px-2 py-0.5 text-xs font-medium text-emerald-600 dark:text-emerald-400">
            {entry.version ? `Installed ${entry.version}` : "Installed"}
          </span>
        ) : (
          <span className="inline-flex shrink-0 items-center rounded-md bg-muted px-2 py-0.5 text-xs font-medium text-muted-foreground">
            Not responding
          </span>
        )}
      </div>

      {entry.description ? (
        <p className="text-xs text-muted-foreground">{entry.description}</p>
      ) : null}

      {/*
        Clamped: this is the provider's own stderr, capped at 4 KiB by
        `invoke_provider` and otherwise unbounded. A wrapper script dumping a
        stack trace into a ~240px column would otherwise grow this card to
        thousands of pixels and push every section below it off screen. Three
        lines is the repo's idiom for an untrusted long string.
      */}
      {entry.error ? (
        <p className="line-clamp-3 wrap-break-word text-xs text-muted-foreground">
          {entry.error}
        </p>
      ) : null}

      {/*
        The trust warning travels with every surface that names a provider.
        A provider binary receives the agent's private key at deploy time
        (`WhereToRunSection`), so a gallery that renders providers as installed
        capabilities without it would quietly promote them to blessed.
      */}
      {isReady ? (
        <div className="flex gap-2 rounded-xl border border-warning/30 bg-warning-bg px-3 py-2">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-warning" />
          <p className="text-xs text-warning">
            This provider receives your agent&apos;s private key when it
            deploys. Only use providers from trusted sources.
          </p>
        </div>
      ) : null}
    </div>
  );
}

/**
 * No provider installed — the common case, and the one that has to teach.
 *
 * The hint sentence is the shared `NO_BACKEND_PROVIDER_HINT` the create dialog
 * and the onboarding notice render, so the same fact is stated once in one
 * vocabulary; this surface then adds the detail the others have no room for.
 */
function RemoteServersEmptyState() {
  return (
    <div
      className="rounded-2xl bg-muted/20 px-4 py-4 text-sm text-muted-foreground"
      data-testid="remote-server-empty"
    >
      <p>{NO_BACKEND_PROVIDER_HINT}</p>
      <p className="mt-2 text-xs">
        Providers are separate binaries named{" "}
        <span className="font-mono">buzz-backend-&lt;id&gt;</span>, discovered
        on your <span className="font-mono">PATH</span> and in{" "}
        <span className="font-mono">~/.local/bin</span>. They are not bundled
        with Buzz: installing one is how you grant this computer the ability to
        deploy agents elsewhere.
      </p>
    </div>
  );
}
