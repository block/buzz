import type { ManagedAgentBackend } from "@/shared/api/types";

/**
 * A single saved provider-config row, ready to render.
 *
 * `value` is always a display string: scalars are stringified, `null` becomes
 * an explicit "Not set", and anything non-scalar (array/object) is summarized
 * rather than dumped — a generic "render every value" helper must not become
 * a disclosure surface for config shapes we have never seen.
 */
export type RunOnConfigRow = {
  key: string;
  label: string;
  value: string;
  /** True when the key looks secret-shaped and the value was replaced. */
  redacted: boolean;
};

export type RunOnSummary =
  | { location: "local" }
  | { location: "provider"; providerId: string; rows: RunOnConfigRow[] };

/**
 * Words that mark a key's value as unrenderable, checked against the same
 * word-split the create-time gate uses (`validate_provider_config`,
 * src-tauri managed_agents/backend.rs) so there is one definition of
 * "looks like a secret", not two that drift. That gate already rejects
 * secret-shaped keys on the only non-test write path to `agent.backend` —
 * this display-side redaction is screenshot hygiene, not the missing
 * credential fix: a value that is fine in `managed-agents.json` on the
 * owner's own disk is not fine in a dialog that gets screenshotted into a
 * channel or PR body, and hand-edited records never met the gate at all.
 * The first five words mirror the Rust list; the rest are display-only
 * extras (redacting more than create rejects fails safe).
 */
const SECRET_WORDS = new Set([
  "secret",
  "password",
  "token",
  "key",
  "credential",
  "passphrase",
  "auth",
  "nsec",
]);

const REDACTED_PLACEHOLDER = "••••••••";

/** Acronyms that read wrong in sentence case ("Cpu request"). */
const LABEL_ACRONYMS = new Set(["cpu", "id", "url", "api"]);

/**
 * Split a config key into lowercase words on `_`, `-`, `.`, and camelCase
 * boundaries, keeping acronym runs together ("APIKey" → ["api", "key"]).
 * Mirrors `split_config_key` in managed_agents/backend.rs — one split feeds
 * both labels and redaction, and word matching (not substring) is what
 * keeps "keyboard" from being treated as a key.
 */
function splitConfigKey(key: string): string[] {
  return key
    .replace(/([a-z0-9])([A-Z])/g, "$1 $2")
    .replace(/([A-Z]+)([A-Z][a-z])/g, "$1 $2")
    .split(/[_\s.-]+/)
    .filter((word) => word.length > 0)
    .map((word) => word.toLowerCase());
}

/**
 * Humanize a stored config key: `cpu_request` → "CPU request". Labels come
 * from the saved keys, not a live provider probe — probing during edit is
 * executable work whose schema (titles, generated defaults) reflects the
 * plugin *today*, not what this agent was deployed with. See PR #4411 for
 * why dialogs must not re-probe as a side effect.
 */
export function humanizeConfigKey(key: string): string {
  const words = splitConfigKey(key);
  if (words.length === 0) return key;
  return words
    .map((word, index) => {
      if (LABEL_ACRONYMS.has(word)) return word.toUpperCase();
      if (index === 0) return word.charAt(0).toUpperCase() + word.slice(1);
      return word;
    })
    .join(" ");
}

function displayValue(value: unknown): string {
  if (value === null || value === undefined) return "Not set";
  if (typeof value === "string") return value.length > 0 ? value : "Not set";
  if (typeof value === "number" || typeof value === "boolean")
    return String(value);
  // Arrays/objects: summarize, never serialize. A nested structure could
  // carry values its own keys would have redacted.
  return Array.isArray(value)
    ? `List (${value.length} items)`
    : "Structured value";
}

/**
 * Preferred display order, traced from the Kubernetes provider schema
 * (crates/buzz-backend-kubernetes/src/config.rs): locate the deployment
 * first (context → namespace), then what runs (image), then the
 * request/limit pairs kept adjacent, then lifecycle/identity. Keys not
 * listed here (other providers, future fields) spill over alphabetically
 * after the known ones, so ordering stays deterministic for every record.
 */
const PREFERRED_KEY_ORDER = [
  "context",
  "namespace",
  "image",
  "cpu_request",
  "memory_request",
  "cpu_limit",
  "memory_limit",
  "inactivity_seconds",
  "service_account",
];

function compareKeys(a: string, b: string): number {
  const ia = PREFERRED_KEY_ORDER.indexOf(a);
  const ib = PREFERRED_KEY_ORDER.indexOf(b);
  if (ia !== -1 && ib !== -1) return ia - ib;
  if (ia !== -1) return -1;
  if (ib !== -1) return 1;
  return a.localeCompare(b);
}

/**
 * Project a `ManagedAgentBackend` into renderable rows. These are the
 * *saved* settings — the config recorded when the agent was created — not
 * the provider's effective settings: optional fields a record omits are
 * defaulted by the provider at deploy time, and synthesizing today's plugin
 * defaults here could show values that drifted from what actually deployed.
 * (`backendAgentId` is deliberately not shown: it is deploy-time runtime
 * state written on start, not saved creation intent, and this section's
 * contract is the latter.) Rows follow the preferred key order with
 * alphabetical spillover, so rendering is deterministic regardless of the
 * JSON key order a given record happened to persist.
 */
export function summarizeRunOn(backend: ManagedAgentBackend): RunOnSummary {
  if (backend.type === "local") return { location: "local" };
  const rows = Object.entries(backend.config ?? {})
    .sort(([a], [b]) => compareKeys(a, b))
    .map(([key, value]): RunOnConfigRow => {
      const redacted = splitConfigKey(key).some((word) =>
        SECRET_WORDS.has(word),
      );
      return {
        key,
        label: humanizeConfigKey(key),
        value: redacted ? REDACTED_PLACEHOLDER : displayValue(value),
        redacted,
      };
    });
  return { location: "provider", providerId: backend.id, rows };
}
