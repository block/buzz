/**
 * Pure helper functions for displaying baked build env values in the global
 * agent config card. Extracted into their own module so unit tests can import
 * them without pulling in React, Tauri IPC, or TanStack Query.
 */

/**
 * Return the provider option label for the zero-value (inherit) option when a
 * baked provider is present. Falls back to the raw provider id when the id
 * doesn't appear in the options table.
 *
 * Used in GlobalAgentConfigSettingsCard to relabel the provider dropdown's
 * empty-selection option when a baked build provider is set.
 */
export function getBakedProviderInheritLabel(
  bakedProviderId: string,
  options: readonly { id: string; label: string }[],
): string {
  const match = options.find((o) => o.id === bakedProviderId);
  const friendlyName = match ? match.label : bakedProviderId;
  return `${friendlyName} (inherited from build)`;
}
