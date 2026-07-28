/**
 * Pure logic for the consolidated Harnesses settings surface and the
 * Add-runtimes catalog dialog.
 *
 * Extracted for deterministic unit-testing — no React, no Tauri, no network.
 */

import type { AcpRuntimeCatalogEntry } from "@/shared/api/types";

// Builtins that anchor the top of "Your runtimes" — mirrors the old
// DoctorSettingsPanel RUNTIME_SORT_PRIORITY so the Buzz + Goose rows stay
// where users learned to find them.
const ROW_SORT_PRIORITY: Record<string, number> = {
  "buzz-agent": 0,
  goose: 1,
};

/**
 * True when the entry earns a row in "Your runtimes":
 *
 * - it is ready (`availability === "available"`), or
 * - one click of the Install button makes it ready (auto-install works), or
 * - the user authored it (`source === "custom"` — owner rows keep their
 *   edit/delete affordances regardless of readiness).
 *
 * Everything else needs multi-step setup and belongs in the Add-runtimes
 * catalog with a real setup action — NOT an inert row control.
 */
export function isYourHarnessEntry(entry: AcpRuntimeCatalogEntry): boolean {
  if (entry.source === "custom") return true;
  if (entry.availability === "available") return true;
  return entry.canAutoInstall && !entry.nodeRequired;
}

/** Entries that render as rows in "Your runtimes". */
export function yourHarnessEntries(
  catalog: readonly AcpRuntimeCatalogEntry[],
): AcpRuntimeCatalogEntry[] {
  return catalog.filter(isYourHarnessEntry);
}

/**
 * Entries offered in the Add-runtimes catalog: every non-custom entry.
 * Ready ones still show (marked as such) so the catalog doubles as a
 * browsable inventory, like the Agent Catalog.
 */
export function catalogDialogEntries(
  catalog: readonly AcpRuntimeCatalogEntry[],
): AcpRuntimeCatalogEntry[] {
  return catalog
    .filter((e) => e.source !== "custom")
    .sort(compareCatalogEntries);
}

/** Needs-setup entries first (that's why the user opened the dialog), then
 * ready ones; alphabetical within each group. */
function compareCatalogEntries(
  a: AcpRuntimeCatalogEntry,
  b: AcpRuntimeCatalogEntry,
): number {
  const aReady = a.availability === "available" ? 1 : 0;
  const bReady = b.availability === "available" ? 1 : 0;
  if (aReady !== bReady) return aReady - bReady;
  return a.label.localeCompare(b.label);
}

/**
 * Splits catalog entries into the two accordion sections of the Add-runtimes
 * list: "Setup" (needs action — the reason the user opened the dialog) and
 * "Installed" (already ready, collapsed by default). Relative order within
 * each group is preserved from the input.
 */
export function groupCatalogEntries(
  entries: readonly AcpRuntimeCatalogEntry[],
): {
  setup: AcpRuntimeCatalogEntry[];
  installed: AcpRuntimeCatalogEntry[];
} {
  return {
    setup: entries.filter((e) => e.availability !== "available"),
    installed: entries.filter((e) => e.availability === "available"),
  };
}

/** Case-insensitive catalog search across label, id, and command. */
export function filterCatalogEntries(
  entries: readonly AcpRuntimeCatalogEntry[],
  query: string,
): AcpRuntimeCatalogEntry[] {
  const q = query.trim().toLowerCase();
  if (!q) return [...entries];
  return entries.filter(
    (e) =>
      e.label.toLowerCase().includes(q) ||
      e.id.toLowerCase().includes(q) ||
      (e.command ?? "").toLowerCase().includes(q),
  );
}

function compareInitialRows(
  a: AcpRuntimeCatalogEntry,
  b: AcpRuntimeCatalogEntry,
): number {
  const aPriority = ROW_SORT_PRIORITY[a.id] ?? Number.MAX_SAFE_INTEGER;
  const bPriority = ROW_SORT_PRIORITY[b.id] ?? Number.MAX_SAFE_INTEGER;
  if (aPriority !== bPriority) return aPriority - bPriority;
  const aOn = a.availability === "available" ? 0 : 1;
  const bOn = b.availability === "available" ? 0 : 1;
  if (aOn !== bOn) return aOn - bOn;
  return a.label.localeCompare(b.label);
}

/**
 * Stable row ordering for "Your runtimes".
 *
 * First render sorts priority builtins first, then ready-before-needs-setup,
 * then alphabetically. Subsequent renders KEEP the previous relative order
 * for ids that are still present — a row that just finished installing must
 * not jump around under the pointer — and append newcomers (e.g. a harness
 * just added from the catalog) using the initial comparator.
 *
 * Returns the ordered id list; callers map ids back to entries.
 */
export function stableRowOrder(
  previousOrder: readonly string[],
  entries: readonly AcpRuntimeCatalogEntry[],
): string[] {
  const present = new Set(entries.map((e) => e.id));
  const kept = previousOrder.filter((id) => present.has(id));
  const keptSet = new Set(kept);
  const appended = entries
    .filter((e) => !keptSet.has(e.id))
    .sort(compareInitialRows)
    .map((e) => e.id);
  return [...kept, ...appended];
}

/** Human status label for a catalog entry; null when nothing needs saying. */
export function entryStatusLabel(entry: AcpRuntimeCatalogEntry): string | null {
  if (entry.authStatus.status === "config_invalid") return "Config error";
  switch (entry.availability) {
    case "adapter_missing":
      return "Adapter needed";
    case "adapter_outdated":
      return "Update needed";
    case "cli_missing":
    case "not_installed":
      return "CLI needed";
    case "available":
      return entry.authStatus.status === "logged_out" ? "Sign-in needed" : null;
    default:
      return null;
  }
}

/**
 * Body copy for the confirmation dialog shown before replacing an
 * already-installed (but outdated) adapter.
 *
 * Codex carries a specific machine-wide caveat about the legacy Zed adapter
 * contract; every other runtime gets generic, runtime-derived copy — Codex
 * package names must never appear for another runtime.
 */
export function adapterUpdateWarning(entry: AcpRuntimeCatalogEntry): string {
  if (entry.id === "codex") {
    return (
      "This replaces the machine-wide codex-acp adapter. Older Buzz " +
      "releases using the legacy adapter may lose community access until " +
      "@zed-industries/codex-acp@0.16.0 is restored."
    );
  }
  const adapter = entry.command?.trim() || entry.label;
  return (
    `This replaces the machine-wide ${adapter} adapter. Other tools using ` +
    "the currently installed adapter will switch to the updated version."
  );
}

export type CatalogPrimaryAction =
  | { kind: "install"; label: string }
  | { kind: "docs"; label: string }
  | { kind: "none" };

/**
 * True when the vendor's install link is a plain download page (e.g.
 * cursor.com/downloads, kimi.ai/download) rather than written setup docs —
 * "Setup guide" would over-promise for those.
 */
export function isDownloadPageUrl(url: string): boolean {
  try {
    return /download/i.test(new URL(url.trim()).pathname);
  } catch {
    return false;
  }
}

/** Label for a link that opens `installInstructionsUrl`. */
export function installLinkLabel(entry: AcpRuntimeCatalogEntry): string {
  return isDownloadPageUrl(entry.installInstructionsUrl)
    ? "Download page"
    : "Setup guide";
}

/**
 * Primary action for the catalog detail pane.
 *
 * - Ready → no action (the entry already has a row in Your runtimes).
 * - One-click installable → Install.
 * - Otherwise → open the vendor's setup guide or download page, when one
 *   exists.
 */
export function catalogPrimaryAction(
  entry: AcpRuntimeCatalogEntry,
): CatalogPrimaryAction {
  if (entry.availability === "available") return { kind: "none" };
  if (entry.canAutoInstall && !entry.nodeRequired) {
    return {
      kind: "install",
      label: entry.availability === "adapter_outdated" ? "Update" : "Install",
    };
  }
  if (entry.installInstructionsUrl.trim().length > 0) {
    return { kind: "docs", label: installLinkLabel(entry) };
  }
  return { kind: "none" };
}
