/**
 * Utilities for extracting and parsing the `buzz:config-nudge` sentinel that
 * `buzz-acp`'s setup-listener appends to its kind:9 nudge body.
 *
 * Wire format (appended by `setup_mode.rs::nudge_body()`):
 *
 * ```
 * ```buzz:config-nudge
 * {"agent_name":"…","agent_pubkey":"…","requirements":[…]}
 * ```
 * ```
 *
 * The prose above the fence is left untouched and used as a plaintext
 * fallback for non-card clients. The desktop detects the sentinel here,
 * strips it from the displayed markdown, and renders a `ConfigNudgeCard`.
 */

// ── Types ─────────────────────────────────────────────────────────────────────

import type { AcpAvailabilityStatus } from "@/shared/api/types";

/** A single missing requirement — mirrors the Rust `RequirementPayload` enum. */
export type ConfigNudgeRequirement =
  | { surface: "normalized_field"; field: string }
  | { surface: "env_key"; key: string }
  | {
      surface: "cli_login";
      probe_args: string[];
      setup_copy: string;
      /**
       * Granular install/auth state — mirrors `AcpAvailabilityStatus` from Rust.
       * Determines which message and CTA the nudge card shows:
       * - "available"         → tooling installed, needs login
       * - "adapter_missing"   → CLI installed but ACP adapter missing
       * - "cli_missing"       → ACP adapter installed but CLI missing
       * - "not_installed"     → neither adapter nor CLI found
       */
      availability: AcpAvailabilityStatus;
    }
  | {
      /**
       * The CLI is installed but its config file could not be parsed.
       * Informational only — no in-app action can repair an external config
       * file. Rendered without a Doctor or Edit Agent CTA.
       */
      surface: "cli_config_invalid";
      probe_args: string[];
      setup_copy: string;
      /** One-line stderr excerpt from the CLI's parse error. */
      diagnostic: string;
    }
  | { surface: "git_bash" };

/**
 * The structured payload embedded in the `buzz:config-nudge` sentinel block.
 * Mirrors the Rust `SetupPayload` struct.
 */
export type ConfigNudgePayload = {
  agent_name: string;
  /** Hex-encoded agent pubkey. Used by the desktop card action to open Edit Agent. */
  agent_pubkey: string;
  requirements: ConfigNudgeRequirement[];
};

// ── Constants ─────────────────────────────────────────────────────────────────

const FENCE_OPEN = "```buzz:config-nudge";
const FENCE_CLOSE = "```";

// ── Extractor ─────────────────────────────────────────────────────────────────

/**
 * Extract the `ConfigNudgePayload` from a message body, if present.
 *
 * Returns `null` when:
 * - the sentinel fence is absent
 * - the JSON inside is malformed
 * - the parsed value doesn't match the expected shape
 *
 * Never throws — all errors are swallowed so this is safe to call in the
 * render path.
 */
export function extractConfigNudge(content: string): ConfigNudgePayload | null {
  const openIdx = content.indexOf(FENCE_OPEN);
  if (openIdx === -1) return null;

  // The JSON starts on the line after the opening fence.
  const jsonStart = content.indexOf("\n", openIdx);
  if (jsonStart === -1) return null;

  // The JSON ends at the next closing ``` that appears on its own line.
  const closeIdx = content.indexOf(`\n${FENCE_CLOSE}`, jsonStart);
  if (closeIdx === -1) return null;

  const json = content.slice(jsonStart + 1, closeIdx).trim();
  if (!json) return null;

  try {
    const parsed: unknown = JSON.parse(json);
    return isConfigNudgePayload(parsed) ? parsed : null;
  } catch {
    return null;
  }
}

/**
 * Strip the `buzz:config-nudge` sentinel block (and any preceding blank line)
 * from a message body. Returns the original string unchanged when no sentinel
 * is present.
 *
 * Used so the prose fallback is rendered without the raw code block.
 */
export function stripConfigNudgeSentinel(content: string): string {
  const openIdx = content.indexOf(FENCE_OPEN);
  if (openIdx === -1) return content;

  const closeIdx = content.indexOf(`\n${FENCE_CLOSE}`, openIdx);
  if (closeIdx === -1) return content;

  const afterFence = closeIdx + `\n${FENCE_CLOSE}`.length;
  // Trim a preceding blank line so the prose doesn't gain a trailing gap.
  const prose = content.slice(0, openIdx).replace(/\n{2,}$/, "\n");
  return prose + content.slice(afterFence);
}

// ── Type-guard ─────────────────────────────────────────────────────────────────

function isConfigNudgeRequirement(v: unknown): v is ConfigNudgeRequirement {
  if (typeof v !== "object" || v === null) return false;
  const r = v as Record<string, unknown>;
  if (typeof r.surface !== "string") return false;
  switch (r.surface) {
    case "normalized_field":
      return typeof r.field === "string";
    case "env_key":
      return typeof r.key === "string";
    case "cli_login":
      return (
        Array.isArray(r.probe_args) &&
        r.probe_args.every((a) => typeof a === "string") &&
        typeof r.setup_copy === "string" &&
        (r.availability === "available" ||
          r.availability === "adapter_missing" ||
          r.availability === "cli_missing" ||
          r.availability === "not_installed")
      );
    case "git_bash":
      return true;
    case "cli_config_invalid":
      return (
        Array.isArray(r.probe_args) &&
        r.probe_args.every((a) => typeof a === "string") &&
        typeof r.setup_copy === "string" &&
        typeof r.diagnostic === "string"
      );
    default:
      return false;
  }
}

function isConfigNudgePayload(v: unknown): v is ConfigNudgePayload {
  if (typeof v !== "object" || v === null) return false;
  const p = v as Record<string, unknown>;
  return (
    typeof p.agent_name === "string" &&
    typeof p.agent_pubkey === "string" &&
    Array.isArray(p.requirements) &&
    p.requirements.every(isConfigNudgeRequirement)
  );
}
