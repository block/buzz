import * as React from "react";
import { AlertTriangle, Loader2, RefreshCw } from "lucide-react";

import {
  connectRemoteAgent,
  listSshHosts,
  probeAgentHost,
} from "@/shared/api/remoteAgentApi";
import type { ConnectedAgent, SshHost } from "@/shared/api/remoteAgentTypes";
import { Button } from "@/shared/ui/button";
import {
  Dialog,
  DialogContent,
  DialogDescription,
  DialogHeader,
  DialogTitle,
} from "@/shared/ui/dialog";
import { Input } from "@/shared/ui/input";
import {
  canSubmitConnectAgent,
  connectAgentPayload,
  emptyConnectAgentDraft,
  harnessOptions,
  missingBuzzCli,
  nameInputMessage,
  pubkeyInputMessage,
} from "./connectAgentIntent";

const SELECT_CLASS =
  "flex h-9 w-full rounded-md border border-input bg-background px-3 py-2 text-sm shadow-xs";

/**
 * Connect a self-hosted agent: one that already runs on a machine the user
 * owns and holds its own key.
 *
 * The wording throughout is "connect", never "create" or "add". Every other
 * agent dialog in Buzz mints an identity and takes over a process; this one
 * records an identity that already exists. Conflating the two would promise
 * lifecycle control that does not exist here.
 */
export function ConnectAgentDialog({
  open,
  onConnected,
  onOpenChange,
}: {
  open: boolean;
  onConnected: (agent: ConnectedAgent) => void;
  onOpenChange: (open: boolean) => void;
}) {
  const [draft, setDraft] = React.useState(emptyConnectAgentDraft);
  const [hosts, setHosts] = React.useState<SshHost[]>([]);
  const [hostsLoaded, setHostsLoaded] = React.useState(false);
  const [error, setError] = React.useState<string | null>(null);
  const [isSubmitting, setIsSubmitting] = React.useState(false);

  React.useEffect(() => {
    if (!open) return;
    let cancelled = false;
    void listSshHosts()
      .then((result) => {
        if (cancelled) return;
        setHosts(result);
        setHostsLoaded(true);
        setDraft((current) =>
          current.host || result.length === 0
            ? current
            : { ...current, host: result[0].host },
        );
      })
      .catch(() => {
        if (!cancelled) setHostsLoaded(true);
      });
    return () => {
      cancelled = true;
    };
  }, [open]);

  const runProbe = React.useCallback((host: string) => {
    if (!host) return;
    setDraft((current) => ({ ...current, isProbing: true, probe: null }));
    void probeAgentHost(host)
      .then((probe) => {
        setDraft((current) =>
          // A stale probe must not overwrite a newer host selection — the user
          // may have switched machines while a slow ssh handshake was open.
          current.host === host
            ? { ...current, probe, isProbing: false }
            : current,
        );
      })
      .catch(() => {
        setDraft((current) =>
          current.host === host
            ? { ...current, probe: null, isProbing: false }
            : current,
        );
      });
  }, []);

  // Probe on host change only — deliberately not on every draft edit, which
  // would open an ssh connection per keystroke. The probe is what fills the
  // harness options and it is read-only on the host, so running it
  // automatically costs the user nothing they did not ask for by picking a
  // machine.
  React.useEffect(() => {
    if (!open || !draft.host) return;
    runProbe(draft.host);
  }, [draft.host, open, runProbe]);

  function reset() {
    setDraft(emptyConnectAgentDraft);
    setError(null);
    setIsSubmitting(false);
  }

  function handleOpenChange(next: boolean) {
    if (!next) reset();
    onOpenChange(next);
  }

  async function handleSubmit() {
    const payload = connectAgentPayload(draft);
    if (!payload) return;
    setIsSubmitting(true);
    setError(null);
    try {
      const agent = await connectRemoteAgent(payload);
      onConnected(agent);
      handleOpenChange(false);
    } catch (cause) {
      setError(cause instanceof Error ? cause.message : String(cause));
    } finally {
      setIsSubmitting(false);
    }
  }

  const readyHarnesses = harnessOptions(draft.probe);
  const pubkeyProblem = pubkeyInputMessage(draft.pubkey);
  const nameProblem = nameInputMessage(draft.name);

  return (
    <Dialog onOpenChange={handleOpenChange} open={open}>
      <DialogContent className="sm:max-w-lg">
        <DialogHeader>
          <DialogTitle>Connect an agent</DialogTitle>
          <DialogDescription>
            Point Buzz at an agent that already runs on one of your machines.
            The agent keeps its own key and supervises itself — Buzz only talks
            to it.
          </DialogDescription>
        </DialogHeader>

        <div className="space-y-4">
          <div className="space-y-1.5">
            <label className="text-sm font-medium" htmlFor="connect-agent-host">
              Machine
            </label>
            {hostsLoaded && hosts.length === 0 ? (
              <p className="text-sm text-muted-foreground">
                No hosts in <span className="font-mono">~/.ssh/config</span>.
                Buzz reaches self-hosted agents through your own ssh config —
                add a <span className="font-mono">Host</span> stanza for the
                machine and reopen this dialog.
              </p>
            ) : (
              <div className="flex gap-2">
                <select
                  className={SELECT_CLASS}
                  disabled={isSubmitting}
                  id="connect-agent-host"
                  onChange={(event) =>
                    setDraft((current) => ({
                      ...current,
                      host: event.target.value,
                      harness: "",
                    }))
                  }
                  value={draft.host}
                >
                  {hosts.map((host) => (
                    <option key={host.host} value={host.host}>
                      {host.host}
                      {host.hostname ? ` — ${host.hostname}` : ""}
                    </option>
                  ))}
                </select>
                <Button
                  aria-label="Re-check this machine"
                  disabled={draft.isProbing || !draft.host}
                  onClick={() => runProbe(draft.host)}
                  size="icon"
                  type="button"
                  variant="outline"
                >
                  {draft.isProbing ? (
                    <Loader2 className="animate-spin" />
                  ) : (
                    <RefreshCw />
                  )}
                </Button>
              </div>
            )}
            <HostProbeSummary draft={draft} />
          </div>

          <div className="space-y-1.5">
            <label
              className="text-sm font-medium"
              htmlFor="connect-agent-pubkey"
            >
              Agent identity
            </label>
            <Input
              autoComplete="off"
              disabled={isSubmitting}
              id="connect-agent-pubkey"
              onChange={(event) =>
                setDraft((current) => ({
                  ...current,
                  pubkey: event.target.value,
                }))
              }
              placeholder="npub1… or 64 hex characters"
              spellCheck={false}
              value={draft.pubkey}
            />
            {pubkeyProblem ? (
              <p className="text-sm text-destructive">{pubkeyProblem}</p>
            ) : (
              <p className="text-sm text-muted-foreground">
                The agent&apos;s public key. Run{" "}
                <span className="font-mono">buzz users me</span> on the machine
                to read it.
              </p>
            )}
          </div>

          <div className="space-y-1.5">
            <label className="text-sm font-medium" htmlFor="connect-agent-name">
              Name
            </label>
            <Input
              disabled={isSubmitting}
              id="connect-agent-name"
              onChange={(event) =>
                setDraft((current) => ({
                  ...current,
                  name: event.target.value,
                }))
              }
              placeholder="Scout"
              value={draft.name}
            />
            {nameProblem ? (
              <p className="text-sm text-destructive">{nameProblem}</p>
            ) : null}
          </div>

          {readyHarnesses.length > 0 ? (
            <div className="space-y-1.5">
              <label
                className="text-sm font-medium"
                htmlFor="connect-agent-harness"
              >
                Harness
              </label>
              <select
                className={SELECT_CLASS}
                disabled={isSubmitting}
                id="connect-agent-harness"
                onChange={(event) =>
                  setDraft((current) => ({
                    ...current,
                    harness: event.target.value,
                  }))
                }
                value={draft.harness}
              >
                <option value="">Not recorded</option>
                {readyHarnesses.map((harness) => (
                  <option key={harness.id} value={harness.id}>
                    {harness.label}
                    {harness.version ? ` ${harness.version}` : ""}
                  </option>
                ))}
              </select>
              <p className="text-sm text-muted-foreground">
                Recorded for reference. Buzz never runs it — the agent starts
                itself.
              </p>
            </div>
          ) : null}

          {error ? (
            <p className="rounded-2xl border border-destructive/30 bg-destructive/10 px-4 py-3 text-sm text-destructive">
              {error}
            </p>
          ) : null}
        </div>

        <div className="flex justify-end gap-2">
          <Button
            disabled={isSubmitting}
            onClick={() => handleOpenChange(false)}
            type="button"
            variant="ghost"
          >
            Cancel
          </Button>
          <Button
            disabled={!canSubmitConnectAgent(draft) || isSubmitting}
            onClick={() => {
              void handleSubmit();
            }}
            type="button"
          >
            {isSubmitting ? "Connecting…" : "Connect"}
          </Button>
        </div>
      </DialogContent>
    </Dialog>
  );
}

/**
 * What the host probe found, or why it could not say.
 *
 * A probe failure is reported but never blocks the connect: a machine that is
 * asleep or off the VPN is still an agent host worth recording.
 */
function HostProbeSummary({ draft }: { draft: typeof emptyConnectAgentDraft }) {
  if (draft.isProbing) {
    return (
      <p className="flex items-center gap-2 text-sm text-muted-foreground">
        <Loader2 className="h-3.5 w-3.5 animate-spin" />
        Checking {draft.host}…
      </p>
    );
  }

  const probe = draft.probe;
  if (!probe) return null;

  if (!probe.ok) {
    return (
      <p className="flex gap-2 text-sm text-muted-foreground">
        <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-warning" />
        <span>
          {probe.errorKind === "password_required"
            ? "This machine asked for a password. Buzz only uses key-based ssh — add a key to connect it later. You can still record the agent now."
            : (probe.error ??
              "Could not reach this machine. You can still record the agent now.")}
        </span>
      </p>
    );
  }

  const readyCount = harnessOptions(probe).length;
  return (
    <div className="space-y-1 text-sm text-muted-foreground">
      <p>
        {readyCount === 0
          ? "No known harnesses found."
          : `${readyCount} harness${readyCount === 1 ? "" : "es"} available.`}
        {probe.buzzCliVersion ? ` buzz ${probe.buzzCliVersion}.` : ""}
      </p>
      {missingBuzzCli(probe) ? (
        <p className="flex gap-2">
          <AlertTriangle className="mt-0.5 h-3.5 w-3.5 shrink-0 text-warning" />
          <span>
            The <span className="font-mono">buzz</span> CLI is not on this
            machine&apos;s PATH. The agent needs it to reach the relay.
          </span>
        </p>
      ) : null}
    </div>
  );
}
