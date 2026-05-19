import * as React from "react";
import { invoke } from "@tauri-apps/api/core";
import { Cpu, Users } from "lucide-react";

import { useMeshLlmOffers } from "@/features/settings/hooks/useMeshLlmOffers";
import { useMeshOfferHeartbeat } from "@/features/settings/hooks/useMeshOfferHeartbeat";
import { Switch } from "@/shared/ui/switch";
import { Input } from "@/shared/ui/input";

// ---------------------------------------------------------------------------
// Types matching the Rust mesh_llm::ComputeSharingPrefs / ResourceCaps shape.
// ---------------------------------------------------------------------------

interface ResourceCaps {
  max_vram_mb: number | null;
  max_ram_mb: number | null;
  max_concurrency: number | null;
}

interface ModelOffer {
  id: string;
  label?: string | null;
  context_tokens?: number | null;
}

interface ComputeSharingPrefs {
  enabled: boolean;
  caps: ResourceCaps;
  models: ModelOffer[];
  d_tag: string;
}

interface MeshEndpointInfo {
  endpoint_id: string;
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// Parse a string into Some(n) when it represents a positive integer, or
/// None to clear the cap. Empty string also clears.
function parseCap(raw: string): number | null {
  if (raw.trim() === "") return null;
  const n = Number.parseInt(raw, 10);
  if (Number.isFinite(n) && n >= 0) return n;
  return null;
}

function formatCap(value: number | null): string {
  return value == null ? "" : String(value);
}

// ---------------------------------------------------------------------------
// Card
// ---------------------------------------------------------------------------

export function MeshComputeSettingsCard() {
  const [prefs, setPrefs] = React.useState<ComputeSharingPrefs | null>(null);
  const [endpoint, setEndpoint] = React.useState<MeshEndpointInfo | null>(null);
  const [irohRelayUrl, setIrohRelayUrl] = React.useState<string | null>(null);
  const [error, setError] = React.useState<string | null>(null);
  const [saving, setSaving] = React.useState(false);
  const { offers, error: offersError } = useMeshLlmOffers();

  // While the card is mounted AND the user has compute-sharing enabled,
  // re-publish the offer every ~5 min so its `expires_at` doesn't lapse
  // (see useMeshOfferHeartbeat for the rationale). The heartbeat hook
  // no-ops when `enabled` is false or `irohRelayUrl` is missing.
  useMeshOfferHeartbeat({
    enabled: prefs?.enabled === true,
    irohRelayUrl,
  });

  // Load the persisted prefs + the iroh endpoint identity + the relay's
  // iroh_relay_url on mount. The latter is what the heartbeat republishes
  // against, so we have to know it before the heartbeat fires its first
  // tick.
  React.useEffect(() => {
    let cancelled = false;
    (async () => {
      try {
        const [p, e] = await Promise.all([
          invoke<ComputeSharingPrefs>("mesh_get_sharing_prefs"),
          invoke<MeshEndpointInfo>("mesh_get_endpoint_id"),
        ]);
        if (cancelled) return;
        setPrefs(p);
        setEndpoint(e);
        // Probe the relay's iroh_relay_url so the heartbeat hook has
        // something to publish against. Failures here are non-fatal —
        // the user can still see/edit prefs; we just won't heartbeat.
        try {
          const relayWsUrl = await invoke<string>("get_relay_ws_url");
          const irohUrl = await invoke<string | null>("mesh_relay_iroh_url", {
            relayWsUrl,
          });
          if (!cancelled) setIrohRelayUrl(irohUrl ?? null);
        } catch (probeErr) {
          console.warn("mesh-llm iroh url probe failed:", probeErr);
        }
      } catch (e) {
        if (!cancelled) setError(String(e));
      }
    })();
    return () => {
      cancelled = true;
    };
  }, []);

  const persist = React.useCallback(async (next: ComputeSharingPrefs) => {
    setSaving(true);
    setError(null);
    try {
      // Save first so a failed publish leaves the prefs in a sane state.
      await invoke("mesh_set_sharing_prefs", { prefs: next });
      setPrefs(next);

      // Probe the connected relay for its iroh_relay_url. If it doesn't
      // advertise mesh-LLM at all, the offer can't be published — but the
      // local prefs are still saved (the user might re-connect to a
      // mesh-capable relay later).
      const relayWsUrl = await invoke<string>("get_relay_ws_url");
      const irohUrl = await invoke<string | null>("mesh_relay_iroh_url", {
        relayWsUrl,
      });
      setIrohRelayUrl(irohUrl ?? null);
      if (irohUrl) {
        await invoke("mesh_publish_offer", { irohRelayUrl: irohUrl });
      } else if (next.enabled) {
        setError(
          "Saved locally, but this relay does not advertise iroh_relay_url — your offer will not be visible to other members until the relay is configured for mesh-LLM.",
        );
      }
    } catch (e) {
      setError(String(e));
    } finally {
      setSaving(false);
    }
  }, []);

  if (!prefs) {
    return (
      <section className="min-w-0" data-testid="settings-compute">
        <div className="mb-3">
          <h2 className="text-sm font-semibold tracking-tight">
            Share compute
          </h2>
          <p className="text-sm text-muted-foreground">
            {error ?? "Loading mesh-LLM preferences…"}
          </p>
        </div>
      </section>
    );
  }

  const updateCap = (field: keyof ResourceCaps, raw: string) => {
    persist({
      ...prefs,
      caps: { ...prefs.caps, [field]: parseCap(raw) },
    });
  };

  return (
    <section className="min-w-0" data-testid="settings-compute">
      <div className="mb-3 min-w-0">
        <h2 className="text-sm font-semibold tracking-tight">Share compute</h2>
        <p className="text-sm text-muted-foreground">
          When enabled, other members of this relay can run agents on this
          machine using the limits you set below. Your relay membership is the
          only gate — there is no signup or external account.
        </p>
      </div>

      {error ? (
        <p className="mb-3 rounded bg-destructive/10 px-2 py-1 text-xs text-destructive">
          {error}
        </p>
      ) : null}

      <div className="flex flex-col gap-4">
        {/* ── Master toggle ────────────────────────────────────────── */}
        <div className="flex items-center justify-between gap-4">
          <div className="min-w-0">
            <label
              className="text-sm font-medium"
              htmlFor="mesh-compute-enabled"
            >
              Share this machine's compute
            </label>
            <p className="text-sm text-muted-foreground">
              Publishes a kind:31990 compute-offer event when on; deletes it
              when off.
            </p>
          </div>
          <Switch
            checked={prefs.enabled}
            data-testid="mesh-compute-enabled-toggle"
            disabled={saving}
            id="mesh-compute-enabled"
            onCheckedChange={(checked) =>
              persist({ ...prefs, enabled: checked })
            }
          />
        </div>

        {/* ── Caps ─────────────────────────────────────────────────── */}
        <fieldset className="flex flex-col gap-3" disabled={!prefs.enabled}>
          <legend className="flex items-center gap-2 text-sm font-medium">
            <Cpu className="h-4 w-4 text-muted-foreground" />
            Limits per request
          </legend>

          <div className="grid grid-cols-3 gap-3">
            <div>
              <label
                className="text-xs text-muted-foreground"
                htmlFor="mesh-vram"
              >
                Max VRAM (MB)
              </label>
              <Input
                data-testid="mesh-cap-vram"
                id="mesh-vram"
                inputMode="numeric"
                onChange={(e) => updateCap("max_vram_mb", e.target.value)}
                placeholder="No limit"
                value={formatCap(prefs.caps.max_vram_mb)}
              />
            </div>
            <div>
              <label
                className="text-xs text-muted-foreground"
                htmlFor="mesh-ram"
              >
                Max RAM (MB)
              </label>
              <Input
                data-testid="mesh-cap-ram"
                id="mesh-ram"
                inputMode="numeric"
                onChange={(e) => updateCap("max_ram_mb", e.target.value)}
                placeholder="No limit"
                value={formatCap(prefs.caps.max_ram_mb)}
              />
            </div>
            <div>
              <label
                className="text-xs text-muted-foreground"
                htmlFor="mesh-concurrency"
              >
                Concurrent peers
              </label>
              <Input
                data-testid="mesh-cap-concurrency"
                id="mesh-concurrency"
                inputMode="numeric"
                onChange={(e) => updateCap("max_concurrency", e.target.value)}
                placeholder="1"
                value={formatCap(prefs.caps.max_concurrency)}
              />
            </div>
          </div>
        </fieldset>

        {/* ── Offers visible from other members ──────────────────── */}
        <section>
          <h3 className="mb-2 flex items-center gap-2 text-sm font-medium">
            <Users className="h-4 w-4 text-muted-foreground" />
            Compute offered by other members
          </h3>
          {offersError ? (
            <p className="text-xs text-destructive">{offersError}</p>
          ) : null}
          {offers.length === 0 ? (
            <p className="text-xs text-muted-foreground">
              Nobody else on this relay is currently sharing compute.
            </p>
          ) : (
            <ul className="flex flex-col gap-2" data-testid="mesh-offers-list">
              {offers.map((entry) => (
                <li
                  className="rounded border border-border/60 bg-muted/20 px-3 py-2 text-xs"
                  key={`${entry.pubkey}:${entry.d_tag}`}
                >
                  <div className="font-medium text-foreground">
                    <code className="break-all">
                      {entry.pubkey.slice(0, 16)}…{entry.pubkey.slice(-4)}
                    </code>
                    {" · "}
                    <span className="text-muted-foreground">{entry.d_tag}</span>
                  </div>
                  <div className="mt-0.5 text-muted-foreground">
                    {entry.offer.models.length === 0
                      ? "No models advertised"
                      : entry.offer.models
                          .map((m) => m.label ?? m.id)
                          .join(", ")}
                  </div>
                  <div className="mt-0.5 text-[10px] text-muted-foreground">
                    {entry.offer.caps.max_vram_mb != null
                      ? `${entry.offer.caps.max_vram_mb} MB VRAM · `
                      : ""}
                    {entry.offer.caps.max_concurrency != null
                      ? `${entry.offer.caps.max_concurrency} concurrent`
                      : ""}
                  </div>
                </li>
              ))}
            </ul>
          )}
        </section>

        {/* ── Identity ────────────────────────────────────────────── */}
        {endpoint ? (
          <div className="rounded border border-border/60 bg-muted/30 px-3 py-2 text-xs text-muted-foreground">
            <span className="font-medium text-foreground">
              This device's iroh endpoint:
            </span>{" "}
            <code className="break-all">{endpoint.endpoint_id}</code>
          </div>
        ) : null}
      </div>
    </section>
  );
}
