/**
 * Settings card for the desktop admin console.
 *
 * Lets an operator enter the admin console URL (the value of `BUZZ_ADMIN_HOST`
 * on their relay), then probes it to determine auth mode and whether the
 * current app identity is on the allowlist.
 *
 * Identity boundary: the stateful body is rendered as
 * `<AdminConsoleSettingsSession key={pubkeyHex} ...>` so that React
 * synchronously unmounts A's entire state tree before B is rendered. Logout
 * (pubkeyHex → empty string) renders nothing, so A's probe state, saved
 * origin, and panel are torn down at the render level — not in a passive effect.
 *
 * Renders the full admin panel when probe state is `nip98Authorized` or
 * `disabled`. The `disabled` state means the relay does not require or
 * validate a credential on the admin API — the desktop still signs outgoing
 * requests, but the relay accepts them unconditionally. The panel works the
 * same way in both states.
 */

import { useEffect, useRef, useState } from "react";
import {
  AlertCircle,
  Check,
  CheckCircle2,
  Copy,
  Info,
  LoaderCircle,
} from "lucide-react";
import { Button } from "@/shared/ui/button";
import { Input } from "@/shared/ui/input";
import { SettingsSectionHeader } from "@/features/settings/ui/SettingsSectionHeader";
import { cn } from "@/shared/lib/cn";
import { copyTextToClipboard } from "@/shared/lib/clipboard";
import {
  getAdminOrigin,
  probeAdminOrigin,
  setAdminOrigin,
  type AdminProbeState,
} from "./api";
import { AdminConsolePanel } from "./AdminConsolePanel";
import { useIdentityQuery } from "@/shared/api/hooks";

// ── Probe state → UI copy ─────────────────────────────────────────────────

// ── DeniedBadge — copy-icon button for the pubkey ─────────────────────────

function DeniedBadge({ pubkeyHex }: { pubkeyHex: string }) {
  const [copied, setCopied] = useState(false);
  const resetTimer = useRef<number | undefined>(undefined);
  useEffect(() => () => window.clearTimeout(resetTimer.current), []);

  return (
    <span className="flex flex-col gap-1 text-xs text-destructive">
      <span className="flex items-center gap-1.5">
        <AlertCircle className="h-3.5 w-3.5 shrink-0" />
        Access denied
      </span>
      <span className="text-muted-foreground">
        Your pubkey is not in{" "}
        <code className="font-mono">BUZZ_ADMIN_PUBKEYS</code>. Ask your relay
        operator to add:
      </span>
      <span className="flex min-w-0 items-center gap-1.5 rounded bg-muted px-1.5 py-0.5">
        <code
          className="min-w-0 flex-1 break-all font-mono text-xs"
          data-testid="admin-denied-pubkey"
        >
          {pubkeyHex}
        </code>
        <Button
          aria-label="Copy pubkey"
          data-testid="admin-denied-pubkey-copy"
          onClick={() => {
            copyTextToClipboard(pubkeyHex, "Pubkey copied");
            setCopied(true);
            window.clearTimeout(resetTimer.current);
            resetTimer.current = window.setTimeout(
              () => setCopied(false),
              1500,
            );
          }}
          size="icon-xs"
          type="button"
          variant="ghost"
        >
          {copied ? (
            <Check className="h-3.5 w-3.5" />
          ) : (
            <Copy className="h-3.5 w-3.5" />
          )}
        </Button>
      </span>
      <span className="text-muted-foreground">
        Other possible causes: clock skew &gt; 60 s, relay config mismatch, or
        the relay is running{" "}
        <code className="font-mono">BUZZ_ADMIN_AUTH=token</code> instead of{" "}
        <code className="font-mono">nip98</code>.
      </span>
    </span>
  );
}

type ProbeUiState =
  | { kind: "idle" }
  | { kind: "probing" }
  | { kind: "authorized"; origin: string }
  | { kind: "denied"; pubkeyHex: string }
  | { kind: "tokenMode" }
  | { kind: "disabled"; origin: string }
  | { kind: "notAdminApi" }
  | { kind: "networkOrIntercepted" }
  | { kind: "error"; message: string };

function ProbeStatusBadge({ uiState }: { uiState: ProbeUiState }) {
  if (uiState.kind === "idle") return null;
  if (uiState.kind === "probing") {
    return (
      <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
        Probing…
      </span>
    );
  }
  if (uiState.kind === "authorized") {
    return (
      <span className="flex items-center gap-1.5 text-xs text-emerald-600 dark:text-emerald-400">
        <CheckCircle2 className="h-3.5 w-3.5" />
        Connected
      </span>
    );
  }
  if (uiState.kind === "denied") {
    return <DeniedBadge pubkeyHex={uiState.pubkeyHex} />;
  }
  if (uiState.kind === "tokenMode") {
    return (
      <span className="flex items-center gap-1.5 text-xs text-amber-600 dark:text-amber-400">
        <Info className="h-3.5 w-3.5" />
        Bearer-token mode. Use the web console — the desktop app only supports
        NIP-98 auth.
      </span>
    );
  }
  if (uiState.kind === "disabled") {
    return (
      <span className="flex items-center gap-1.5 text-xs text-amber-600 dark:text-amber-400">
        <Info className="h-3.5 w-3.5" />
        Auth is disabled on this relay. The admin console is accessible without
        a credential.
      </span>
    );
  }
  if (uiState.kind === "notAdminApi") {
    return (
      <span className="flex items-center gap-1.5 text-xs text-muted-foreground">
        <AlertCircle className="h-3.5 w-3.5" />
        No admin API found at this origin. Check the URL matches{" "}
        <code className="font-mono">BUZZ_ADMIN_HOST</code>.
      </span>
    );
  }
  if (uiState.kind === "networkOrIntercepted") {
    return (
      <span className="flex items-center gap-1.5 text-xs text-destructive">
        <AlertCircle className="h-3.5 w-3.5" />
        Could not reach the relay. Check: network, TLS certificate, DNS, or
        whether a VPN/SSO layer (e.g. Cloudflare Access) intercepts this host.
      </span>
    );
  }
  // error
  return (
    <span className="flex items-center gap-1.5 text-xs text-destructive">
      <AlertCircle className="h-3.5 w-3.5" />
      {uiState.message}
    </span>
  );
}

function probeStateToUiState(
  state: AdminProbeState,
  origin: string,
  pubkeyHex: string,
): ProbeUiState {
  switch (state) {
    case "nip98Authorized":
      return { kind: "authorized", origin };
    case "nip98Denied":
      return { kind: "denied", pubkeyHex };
    case "tokenMode":
      return { kind: "tokenMode" };
    case "disabled":
      return { kind: "disabled", origin };
    case "notAdminApi":
      return { kind: "notAdminApi" };
    case "networkOrIntercepted":
      return { kind: "networkOrIntercepted" };
  }
}

// ── Main card ─────────────────────────────────────────────────────────────

export function AdminConsoleSettingsCard() {
  const { data: identity } = useIdentityQuery();
  const pubkeyHex = identity?.pubkey ?? "";

  return (
    <section
      className="flex min-h-0 flex-1 flex-col overflow-y-auto"
      data-testid="settings-admin-console"
    >
      <SettingsSectionHeader
        title="Admin console"
        description="Connect to your relay's deployment admin API. Paste the value of BUZZ_ADMIN_HOST from your relay config."
      />
      {pubkeyHex ? (
        <AdminConsoleSettingsSession key={pubkeyHex} pubkeyHex={pubkeyHex} />
      ) : null}
    </section>
  );
}

// ── Stateful session — keyed by pubkeyHex ─────────────────────────────────
//
// React's `key` prop causes the parent to unmount this component entirely when
// the pubkey changes. That means:
//  - A→B switch: A's entire state tree (originInput, savedOrigin, probeUiState,
//    isSaving, in-flight probes) is destroyed synchronously before B mounts.
//  - Logout (pubkeyHex → ""): the parent renders `null`, so A's state is gone
//    before any new render begins.
//
// This eliminates the passive-effect reset race where the parent rendered with
// B's pubkey and A's stale origin/authorized state for one render cycle.

function AdminConsoleSettingsSession({ pubkeyHex }: { pubkeyHex: string }) {
  const [originInput, setOriginInput] = useState("");
  const [savedOrigin, setSavedOrigin] = useState<string | null>(null);
  const [probeUiState, setProbeUiState] = useState<ProbeUiState>({
    kind: "idle",
  });
  const [isSaving, setIsSaving] = useState(false);

  // In-flight probe abort controller. Does not cancel the Tauri native request
  // (not cancellable), but prevents a stale probe result from updating UI state.
  const probeAbortRef = useRef<AbortController | null>(null);

  // Save/probe context token: captures (pubkey, origin) at the time a save
  // starts. handleSave checks this before committing any state so a delayed
  // save cannot repopulate the wrong session.
  //
  // On unmount, the cleanup effect below sets sessionTokenRef.current = null.
  // Every handleSave continuation leg checks `sessionTokenRef.current !== token`
  // (null !== token object) → returns early on all paths. This is StrictMode-safe:
  // StrictMode's simulated cleanup fires the null assignment, then the re-mount
  // re-arms the ref when the next handleSave sets `sessionTokenRef.current = token`.
  type SessionToken = { pubkey: string; origin: string };
  const sessionTokenRef = useRef<SessionToken | null>(null);

  // Synchronously abort any active probe and reset probe UI state.
  // Call before starting a new probe or on any input change.
  function abortAndResetProbe() {
    probeAbortRef.current?.abort();
    probeAbortRef.current = null;
    setProbeUiState({ kind: "idle" });
  }

  // Null sessionTokenRef on unmount so A's deferred handleSave continuation
  // fails the token check on all legs after A's component is torn down. Paired
  // with the load-saved-origin effect below: that effect has an explicit
  // lint suppression; this cleanup-only effect has no deps and Biome accepts it.
  useEffect(() => {
    return () => {
      sessionTokenRef.current = null;
    };
  }, []);

  // Load saved origin on mount (runs once per session because the component
  // is keyed by pubkeyHex — re-mount = new pubkey).
  // biome-ignore lint/correctness/useExhaustiveDependencies: intentional mount-once effect; identity boundary is the key prop on this component — it unmounts/remounts on pubkey change, so [] is correct.
  useEffect(() => {
    let active = true;
    void (async () => {
      try {
        const saved = await getAdminOrigin(pubkeyHex);
        if (!active) return;
        setSavedOrigin(saved);
        setOriginInput(saved ?? "");
        if (saved) {
          runProbe(saved);
        }
      } catch (e) {
        if (!active) return;
        // Surface storage/signing errors rather than silently degrading.
        setProbeUiState({
          kind: "error",
          message: e instanceof Error ? e.message : String(e),
        });
        setSavedOrigin(null);
        setOriginInput("");
      }
    })();
    return () => {
      active = false;
    };
  }, []); // Empty: runs once per session mount; identity boundary is the key prop.

  function runProbe(origin: string) {
    probeAbortRef.current?.abort();
    const controller = new AbortController();
    probeAbortRef.current = controller;

    setProbeUiState({ kind: "probing" });

    void (async () => {
      try {
        const result = await probeAdminOrigin(origin);
        if (controller.signal.aborted) return;
        setProbeUiState(probeStateToUiState(result.state, origin, pubkeyHex));
      } catch (e) {
        if (controller.signal.aborted) return;
        setProbeUiState({
          kind: "error",
          message: e instanceof Error ? e.message : String(e),
        });
      }
    })();
  }

  async function handleSave() {
    const trimmed = originInput.trim();
    // Capture (pubkey, origin) token at save-start time. The check below
    // ensures a delayed completion cannot write into a different session.
    const token: SessionToken = { pubkey: pubkeyHex, origin: trimmed };
    sessionTokenRef.current = token;

    setIsSaving(true);
    abortAndResetProbe();
    try {
      if (!trimmed) {
        const canonical = await setAdminOrigin(null, pubkeyHex);
        // Discard if the session changed while the native call was in flight.
        if (sessionTokenRef.current !== token) return;
        setSavedOrigin(canonical);
        setProbeUiState({ kind: "idle" });
        return;
      }
      const canonical = await setAdminOrigin(trimmed, pubkeyHex);
      if (sessionTokenRef.current !== token) return;
      setSavedOrigin(canonical);
      if (canonical) {
        runProbe(canonical);
      } else {
        setProbeUiState({ kind: "idle" });
      }
    } catch (e) {
      if (sessionTokenRef.current !== token) return;
      setProbeUiState({
        kind: "error",
        message: e instanceof Error ? e.message : String(e),
      });
    } finally {
      if (sessionTokenRef.current === token) setIsSaving(false);
    }
  }

  const inputChanged = originInput.trim() !== (savedOrigin ?? "");
  const isPanelVisible =
    (probeUiState.kind === "authorized" || probeUiState.kind === "disabled") &&
    savedOrigin !== null;

  return (
    <>
      <div className="mb-6 space-y-3">
        <div className="flex gap-2">
          <Input
            autoComplete="off"
            className="flex-1 font-mono text-sm"
            data-testid="admin-origin-input"
            disabled={isSaving}
            onChange={(e) => {
              setOriginInput(e.target.value);
              // General reset: abort and clear probe state on every input
              // change, not only when state is `probing`. This prevents a
              // stale probe result from a previous value being committed.
              abortAndResetProbe();
            }}
            placeholder="https://admin.yourrelay.example.com"
            spellCheck={false}
            type="url"
            value={originInput}
            onKeyDown={(e) => {
              if (e.key === "Enter") void handleSave();
            }}
          />
          <Button
            data-testid="admin-origin-save"
            disabled={isSaving || !inputChanged}
            onClick={() => void handleSave()}
            size="sm"
            type="button"
            variant={inputChanged ? "default" : "outline"}
          >
            {isSaving ? (
              <LoaderCircle className="h-3.5 w-3.5 animate-spin" />
            ) : (
              "Save"
            )}
          </Button>
          {savedOrigin && (
            <Button
              className={cn(
                "text-xs",
                probeUiState.kind === "probing" && "opacity-50",
              )}
              data-testid="admin-probe-refresh"
              disabled={probeUiState.kind === "probing"}
              onClick={() => runProbe(savedOrigin)}
              size="sm"
              type="button"
              variant="ghost"
            >
              Re-probe
            </Button>
          )}
        </div>

        <div className="min-h-[1.5rem]">
          <ProbeStatusBadge uiState={probeUiState} />
        </div>
      </div>

      {isPanelVisible && savedOrigin && (
        <AdminConsolePanel origin={savedOrigin} pubkey={pubkeyHex} />
      )}
    </>
  );
}
