import type {
  HostProbeResult,
  RemoteHarness,
} from "@/shared/api/remoteAgentTypes";

/**
 * Draft state for the Connect-an-agent dialog.
 *
 * `probe` is the RC3 host probe result, kept in the draft rather than derived
 * on submit because the harness options and the "is this host even reachable"
 * answer both come from it.
 */
export type ConnectAgentDraft = {
  host: string;
  pubkey: string;
  name: string;
  harness: string;
  probe: HostProbeResult | null;
  isProbing: boolean;
};

export const emptyConnectAgentDraft: ConnectAgentDraft = {
  host: "",
  pubkey: "",
  name: "",
  harness: "",
  probe: null,
  isProbing: false,
};

/**
 * Client-side pubkey shape check.
 *
 * The backend is the authority — it normalizes and stores — but repeating the
 * shape check here lets the dialog disable submit and explain why instead of
 * round-tripping to produce an error. `nsec` gets its own answer because
 * "invalid" would not tell a user who just pasted their agent's secret what
 * they actually did.
 */
export type PubkeyVerdict =
  | { kind: "empty" }
  | { kind: "secret" }
  | { kind: "invalid" }
  | { kind: "ok" };

const HEX64 = /^[0-9a-fA-F]{64}$/;
// npub1 + 58 bech32 data characters. Length is checked rather than the checksum:
// the backend verifies the checksum, and a client-side bech32 implementation
// here would be a second decoder to keep correct.
const NPUB = /^npub1[023456789acdefghjklmnpqrstuvwxyz]{58}$/;

export function verifyPubkeyInput(input: string): PubkeyVerdict {
  const trimmed = input.trim();
  if (!trimmed) return { kind: "empty" };
  if (trimmed.startsWith("nsec")) return { kind: "secret" };
  if (HEX64.test(trimmed) || NPUB.test(trimmed)) return { kind: "ok" };
  return { kind: "invalid" };
}

/** Human-readable reason a pubkey input is not usable yet, or `null`. */
export function pubkeyInputMessage(input: string): string | null {
  switch (verifyPubkeyInput(input).kind) {
    case "empty":
      return null;
    case "secret":
      return "That is a secret key. A self-hosted agent's nsec must never leave its own machine — paste its npub instead.";
    case "invalid":
      return "Expected an npub or 64 hex characters.";
    case "ok":
      return null;
  }
}

export const MAX_CONNECTED_NAME_LENGTH = 64;

/** Human-readable reason a name is not usable yet, or `null`. */
export function nameInputMessage(input: string): string | null {
  const trimmed = input.trim();
  if (!trimmed) return null;
  if (trimmed.length > MAX_CONNECTED_NAME_LENGTH) {
    return `Names are limited to ${MAX_CONNECTED_NAME_LENGTH} characters.`;
  }
  return null;
}

/**
 * Harnesses worth offering for a connected agent.
 *
 * Only `ready` ones: an ACP adapter whose vendor CLI is missing starts and then
 * fails at first use, so listing it as the agent's harness would record
 * something known-broken. An empty list is a legitimate answer — the host may
 * run an agent Buzz has no recipe for — which is why the harness field is
 * optional.
 */
export function harnessOptions(probe: HostProbeResult | null): RemoteHarness[] {
  if (!probe?.ok) return [];
  return probe.harnesses.filter((harness) => harness.ready);
}

/**
 * True when the host probe came back but found no `buzz` CLI.
 *
 * Not a blocker: the CLI can be installed after connecting, and a user may be
 * recording an agent they are still setting up. It is the single most useful
 * warning to show, because without it the agent cannot reach the relay at all.
 */
export function missingBuzzCli(probe: HostProbeResult | null): boolean {
  return Boolean(probe?.ok) && !probe?.buzzCliPath;
}

/**
 * Submit gate.
 *
 * Deliberately does NOT require a successful probe. A machine that is asleep,
 * off the VPN, or mid-reboot is still an agent host the user wants recorded —
 * blocking on reachability would make the feature unusable exactly when the
 * user is setting things up. What is required is a host, a well-formed pubkey,
 * and a name; the backend re-validates all three and additionally rejects a
 * host that is not in `~/.ssh/config`.
 */
export function canSubmitConnectAgent(draft: ConnectAgentDraft): boolean {
  if (draft.isProbing) return false;
  if (!draft.host.trim()) return false;
  if (verifyPubkeyInput(draft.pubkey).kind !== "ok") return false;
  const name = draft.name.trim();
  if (!name || name.length > MAX_CONNECTED_NAME_LENGTH) return false;
  return true;
}

/** The payload `connectRemoteAgent` expects, or `null` when not submittable. */
export function connectAgentPayload(draft: ConnectAgentDraft) {
  if (!canSubmitConnectAgent(draft)) return null;
  const harness = draft.harness.trim();
  return {
    host: draft.host.trim(),
    pubkey: draft.pubkey.trim(),
    name: draft.name.trim(),
    harness: harness ? harness : null,
  };
}

/**
 * Compact label for a failed probe, for the Connected Agents list.
 *
 * Every classified kind gets its own wording because they call for different
 * actions, and "machine unreachable" is actively wrong for all but one of them:
 * the host answered in every case except `unreachable`. Labelling an untrusted
 * host key as unreachable would send someone to check the network when the fix
 * is to review a fingerprint — and since Buzz probes with strict host-key
 * checking and never writes `known_hosts`, this label is the only prompt the
 * user gets.
 */
export function reachabilityLabel(probe: HostProbeResult): string {
  switch (probe.errorKind) {
    case "password_required":
      return "needs an ssh key";
    case "host_key_problem":
      return "host key not trusted";
    case "truncated":
      return "probe incomplete · retry";
    case "timed_out":
      return "probe timed out";
    case "unreachable":
      return "machine unreachable";
    default:
      // Unclassified: the backend could not attribute the failure, so naming a
      // specific cause here would be a guess.
      return "probe failed";
  }
}
