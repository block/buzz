/**
 * In-memory custom harness store for the e2e bridge.
 *
 * Extracted as a separate module so the handler logic can be unit-tested
 * independently of the full e2eBridge.ts context (which requires a browser
 * environment and full Playwright setup).
 */
import type { RawAcpRuntimeCatalogEntry } from "../shared/api/tauri.ts";

/** In-memory store for custom harnesses saved via `save_custom_harness`. */
export const mockCustomHarnesses = new Map<string, RawAcpRuntimeCatalogEntry>();

/**
 * Ids removed via `delete_custom_harness` (or vacated by a rename).
 *
 * Needed because a test's `acpRuntimesCatalog` seed is static config, not the
 * mutation store: deleting a seeded row leaves nothing to remove from
 * `mockCustomHarnesses`, so without a tombstone the row would survive the
 * delete and the mock would report success while the UI still shows it.
 */
export const mockDeletedCustomHarnesses = new Set<string>();

/** Reset the store between tests. */
export function resetMockCustomHarnesses(): void {
  mockCustomHarnesses.clear();
  mockDeletedCustomHarnesses.clear();
}

/**
 * Overlay the mutation store onto a seeded catalog.
 *
 * Deleted ids drop out, saved ids replace their seeded entry in place (so a
 * same-id edit updates rather than duplicates), and newly added ids append.
 */
export function mergeMockCustomHarnesses(
  base: RawAcpRuntimeCatalogEntry[],
): RawAcpRuntimeCatalogEntry[] {
  const merged = base.filter(
    (entry) => !mockDeletedCustomHarnesses.has(entry.id),
  );
  for (const entry of mockCustomHarnesses.values()) {
    const index = merged.findIndex((existing) => existing.id === entry.id);
    if (index === -1) {
      merged.push(entry);
    } else {
      merged[index] = entry;
    }
  }
  return merged;
}

/**
 * Handle `save_custom_harness`.
 *
 * Persists the definition into `mockCustomHarnesses` so that the next
 * `discover_acp_providers` call includes it. Mirrors the Rust command's
 * return shape: an `AcpRuntimeCatalogEntry` for the saved harness.
 */
export function handleSaveCustomHarness(args: {
  definition?: {
    id?: string;
    label?: string;
    command?: string;
    args?: string[];
    env?: Record<string, string>;
    installInstructionsUrl?: string;
    installHint?: string;
  };
  originalId?: string | null;
}): RawAcpRuntimeCatalogEntry {
  const def = args.definition ?? {};
  const id = def.id ?? "";
  const originalId = args.originalId ?? null;

  // On rename: remove the old entry so the old id is no longer in the catalog.
  if (originalId && originalId !== id) {
    mockCustomHarnesses.delete(originalId);
    mockDeletedCustomHarnesses.add(originalId);
  }
  // A save resurrects an id that an earlier test step deleted.
  mockDeletedCustomHarnesses.delete(id);

  const entry: RawAcpRuntimeCatalogEntry = {
    id,
    label: def.label ?? id,
    avatar_url: "",
    availability: "not_installed", // PATH not probed in e2e mock
    command: def.command ?? null,
    binary_path: null,
    default_args: def.args ?? [],
    mcp_command: null,
    install_hint: def.installHint ?? "",
    install_instructions_url: def.installInstructionsUrl ?? "",
    can_auto_install: false,
    requires_external_cli: true,
    underlying_cli_path: null,
    node_required: false,
    auth_status: { status: "not_applicable" },
    source: "custom",
    // Omit definition_env when the env map is empty — mirrors Rust's BTreeMap
    // serialization which skips empty maps so the field is absent on the wire.
    definition_env:
      def.env && Object.keys(def.env).length > 0 ? def.env : undefined,
    login_hint: undefined,
  };
  mockCustomHarnesses.set(id, entry);
  return entry;
}

/**
 * Handle `delete_custom_harness`.
 * Removes the harness from the in-memory store. Idempotent (not-found is OK).
 * When `config?.mock?.deleteCustomHarnessError` is set, throws with that message
 * to exercise the UI's inline error path.
 */
export function handleDeleteCustomHarness(
  args: { id?: string },
  config?: { mock?: { deleteCustomHarnessError?: string } } | undefined,
): void {
  const errorMsg = config?.mock?.deleteCustomHarnessError;
  if (errorMsg) {
    throw new Error(errorMsg);
  }
  const id = args.id ?? "";
  mockCustomHarnesses.delete(id);
  mockDeletedCustomHarnesses.add(id);
}
