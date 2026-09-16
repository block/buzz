/**
 * JoinPage — join-by-address: the one-input join.
 *
 * The founder's ruling this implements: desktop pairing does not scale. A
 * stranger with ONLY a phone and the relay address must end up in the room —
 * zero configuration, zero tools, no VPN, no toggles, no second machine.
 *
 * The flow, one input wide:
 *   paste the relay address → the relay serves its own pairing material
 *   (join.json) → accept terms when the community has them → claim the
 *   standing invite with a key made ON THIS PHONE → land in the room.
 *
 * Fail-closed is untouched: no material published → a plain refusal, never a
 * guess; the client talks only to the address it was given; every relay
 * verdict is surfaced verbatim.
 */

import {
  fetchJoinMaterial,
  inviteCodeFromMaterial,
  type JoinMaterial,
} from "@/features/join/join-material";
import { fetchJoinEvent } from "@/features/join/join-event";
import { RoomView } from "@/features/room/RoomView";
import { normalizeRelayAddress, type RelayAddress } from "@/shared/lib/address";
import {
  exportLocalNsec,
  getLocalKeypair,
  signWithLocalKey,
} from "@/shared/lib/local-identity";
import {
  hasNip07Provider,
  signNostrEvent,
  type SignedNostrEvent,
  type UnsignedNostrEvent,
} from "@/shared/lib/nostr-signer";
import { Button } from "@/shared/ui/button";
import * as React from "react";

const ORIGIN_MEMORY_KEY = "buzz.join.origin";

type Phase =
  | { state: "form" }
  | { state: "resolving" }
  | { state: "joined" }
  | { state: "refused"; reason: string };

type PolicyConfig = {
  terms_markdown?: string;
  privacy_markdown?: string;
  age_attestation_required?: boolean;
  version?: string;
};

/**
 * The community's canonical origin, as the relay itself declares it in
 * NIP-11 (`/info`, `push.origin`). The dual-home law, client side: transport
 * may ride the road the stranger was given, but signing (NIP-98 `u`, NIP-42
 * `relay`) names the community's own identity — the relay rejects events
 * signed against an alias host.
 */
async function canonicalOrigins(
  origin: string,
): Promise<{ http: string; ws: string }> {
  try {
    const response = await fetch(`${origin.replace(/\/+$/, "")}/info`, {
      headers: { Accept: "application/nostr+json" },
      signal: AbortSignal.timeout(6000),
    });
    const info = (await response.json()) as { push?: { origin?: string } };
    const ws = info.push?.origin;
    if (ws && (ws.startsWith("wss://") || ws.startsWith("ws://"))) {
      return {
        ws,
        http: ws.replace(/^wss:/, "https:").replace(/^ws:/, "http:"),
      };
    }
  } catch {
    // no /info or no push.origin → the given address is its own identity
  }
  return {
    ws: origin.replace(/^http/, "ws"),
    http: origin,
  };
}

/** Claim an invite against an explicit relay origin with an explicit identity. */
async function claimInvite(options: {
  origin: string;
  canonicalHttp: string;
  code: string;
  policyReceipt?: string;
  localSecretKey?: Uint8Array;
}): Promise<{ status: string }> {
  const { origin, canonicalHttp, code, policyReceipt, localSecretKey } =
    options;
  // SIGN with the canonical origin; TRANSPORT on the road the user was given.
  const url = `${canonicalHttp.replace(/\/+$/, "")}/api/invites/claim`;
  const body = JSON.stringify({
    code,
    policy_receipt: policyReceipt,
  });
  const authorization = await makeLocalCapableAuthHeader(
    url,
    body,
    localSecretKey,
  );
  const response = await fetch(
    `${origin.replace(/\/+$/, "")}/api/invites/claim`,
    {
      method: "POST",
      headers: {
        Authorization: authorization,
        "Content-Type": "application/json",
      },
      body,
      signal: AbortSignal.timeout(15000),
    },
  );
  const json = (await response.json().catch(() => ({}))) as Record<
    string,
    unknown
  >;
  if (!response.ok) {
    const message =
      typeof json.error === "string" ? json.error : `HTTP ${response.status}`;
    throw new Error(message);
  }
  return { status: String(json.status ?? "joined") };
}

/** NIP-98 header that prefers the extension but can sign with the local key. */
async function makeLocalCapableAuthHeader(
  url: string,
  body: string,
  localSecretKey?: Uint8Array,
): Promise<string> {
  const digest = await crypto.subtle.digest(
    "SHA-256",
    new TextEncoder().encode(body),
  );
  const payload = Array.from(new Uint8Array(digest))
    .map((byte) => byte.toString(16).padStart(2, "0"))
    .join("");
  const signed = await signNostrEvent(
    {
      kind: 27235,
      tags: [
        ["u", url],
        ["method", "POST"],
        ["payload", payload],
        ["nonce", crypto.randomUUID()],
      ],
      content: "",
    },
    localSecretKey ? { secretKey: localSecretKey } : { requireNip07: true },
  );
  return `Nostr ${btoa(JSON.stringify(signed))}`;
}

export function JoinPage() {
  const [addressInput, setAddressInput] = React.useState("");
  const [phase, setPhase] = React.useState<Phase>({ state: "form" });
  const [busy, setBusy] = React.useState(false);
  const [material, setMaterial] = React.useState<JoinMaterial | null>(null);
  const [resolved, setResolved] = React.useState<RelayAddress | null>(null);
  const [joinSource, setJoinSource] = React.useState<"event" | "json" | null>(
    null,
  );

  // Same-origin prefill: when this page is served BY a relay (the estate
  // proves it that way), the address is already in the URL bar — the
  // stranger's one input is done before they arrived.
  React.useEffect(() => {
    if (typeof window === "undefined") return;
    const remembered = localStorage.getItem(ORIGIN_MEMORY_KEY);
    const sameOrigin = window.location.protocol.startsWith("http")
      ? window.location.origin
      : null;
    const prefill = sameOrigin ?? remembered ?? "";
    if (prefill) setAddressInput(prefill);
  }, []);

  const usingExtension = hasNip07Provider();
  const signer = React.useCallback(
    async (
      unsigned: Omit<UnsignedNostrEvent, "created_at"> & {
        created_at?: number;
      },
    ): Promise<SignedNostrEvent> => {
      const full = {
        ...unsigned,
        created_at: unsigned.created_at ?? Math.floor(Date.now() / 1000),
      };
      return usingExtension ? signNostrEvent(full) : signWithLocalKey(full);
    },
    [usingExtension],
  );

  const join = async () => {
    const normalized = normalizeRelayAddress(addressInput);
    if (!normalized) {
      setPhase({
        state: "refused",
        reason: "that does not look like a relay address",
      });
      return;
    }
    setBusy(true);
    setPhase({ state: "resolving" });
    try {
      // THE WIRE FIRST: the owner-signed join material (kind 34550) off
      // the relay itself — the path that needs nothing but the URL. The
      // static join.json stays as the fallback for relays that have not
      // published the event yet.
      const fromWire = await fetchJoinEvent(normalized.wsUrl);
      const found =
        fromWire?.material ?? (await fetchJoinMaterial(normalized.origin));
      setJoinSource(fromWire ? "event" : "json");
      if (!found) {
        setPhase({
          state: "refused",
          reason: `${normalized.host} does not offer join-by-address — it has not published pairing material. The app route (Add Community → paste the address) still works.`,
        });
        return;
      }
      const code = inviteCodeFromMaterial(found);
      if (!code) {
        setPhase({
          state: "refused",
          reason: "the community's pairing material is malformed (no invite)",
        });
        return;
      }

      localStorage.setItem(ORIGIN_MEMORY_KEY, normalized.origin);

      // Terms, when the community has them: acceptance precedes the claim.
      let receipt: string | undefined;
      try {
        const policyResponse = await fetch(
          `${normalized.origin.replace(/\/+$/, "")}/api/join-policy`,
          { headers: { Accept: "application/json" } },
        );
        if (policyResponse.ok) {
          const policy = (await policyResponse.json()) as {
            policy?: PolicyConfig;
          };
          if (policy?.policy?.version) {
            const accepted = await fetch(
              `${normalized.origin.replace(/\/+$/, "")}/api/invites/accept-policy`,
              {
                method: "POST",
                headers: { "Content-Type": "application/json" },
                body: JSON.stringify({
                  code,
                  policy_version: policy.policy.version,
                  age_confirmed: false,
                }),
              },
            );
            if (accepted.ok) {
              const body = (await accepted.json()) as { receipt?: string };
              receipt = body.receipt;
            }
          }
        }
      } catch {
        // No policy readable → the relay still enforces whatever it enforces;
        // a refused claim surfaces verbatim below.
      }

      const localSecretKey = usingExtension
        ? undefined
        : getLocalKeypair().secretKey;
      const canonical = await canonicalOrigins(normalized.origin);
      await claimInvite({
        origin: normalized.origin,
        canonicalHttp: canonical.http,
        code,
        policyReceipt: receipt,
        localSecretKey,
      });

      setMaterial(found);
      setResolved({ ...normalized, canonicalRelayUrl: canonical.ws });
      setPhase({ state: "joined" });
    } catch (error) {
      setPhase({
        state: "refused",
        reason: error instanceof Error ? error.message : String(error),
      });
    } finally {
      setBusy(false);
    }
  };

  if (phase.state === "joined" && material && resolved) {
    const local = usingExtension ? null : getLocalKeypair();
    return (
      <RoomView
        channelName={material.default_channel.name ?? "room"}
        channelId={material.default_channel.id}
        rooms={material.rooms}
        communityName={material.community.name ?? ""}
        exportSecret={exportLocalNsec}
        host={material.community.host || resolved.host}
        canonicalRelayUrl={resolved.canonicalRelayUrl}
        npub={local ? local.npub : ""}
        signer={signer}
        wsUrl={resolved.wsUrl}
      />
    );
  }

  return (
    <div
      className="flex min-h-dvh flex-col items-center justify-center bg-zinc-950 px-6 py-12 text-center"
      data-join-source={joinSource ?? undefined}
    >
      <div className="w-full max-w-md space-y-5">
        <div>
          <h1 className="text-xl font-semibold tracking-tight text-white">
            join by address
          </h1>
          <p className="mt-2 text-sm leading-relaxed text-zinc-400">
            One address is the whole invite. Paste the relay you were given —
            the key is made on this phone, the room opens right here. No app, no
            account, no second machine.
          </p>
        </div>

        <input
          className="h-12 w-full rounded-xl border border-zinc-700 bg-zinc-900 px-4 font-mono text-sm text-white placeholder:text-zinc-500 focus:border-zinc-400 focus:outline-none"
          disabled={busy}
          inputMode="url"
          onChange={(event) => setAddressInput(event.target.value)}
          onKeyDown={(event) => {
            if (event.key === "Enter") join();
          }}
          placeholder="wss://relay.example.com"
          spellCheck={false}
          value={addressInput}
        />

        <Button
          className="h-12 w-full bg-white text-black hover:bg-zinc-200"
          disabled={busy || !addressInput.trim()}
          onClick={join}
        >
          {busy
            ? phase.state === "resolving"
              ? "joining…"
              : "join"
            : "join the room →"}
        </Button>

        {phase.state === "refused" ? (
          <p className="rounded-xl border border-amber-800 bg-amber-950/50 px-4 py-3 text-left text-xs leading-relaxed text-amber-300">
            {phase.reason}
          </p>
        ) : null}

        <p className="text-[11px] leading-relaxed text-zinc-500">
          {usingExtension
            ? "Your NIP-07 extension will sign the join."
            : "This browser has no extension, so a key will be generated here and kept in this browser — you can copy it out once you're in."}
        </p>
      </div>
    </div>
  );
}
