import type { BlobDescriptor } from "@/shared/api/tauri";

/**
 * Slot bookkeeping for composer attachments.
 *
 * Attachments live in a sparse array: an immediate upload calls `reserveSlots`
 * to claim an index up front and fills it by that index when it completes, so
 * concurrent uploads publish in the order they were attached. A `null` is a
 * placeholder for an upload still in flight.
 *
 * Consumers of the composer only ever see the compacted list of real
 * attachments, so any update expressed against that view has to be mapped back
 * onto the slot layout — never applied to it directly.
 */

/**
 * Identity of a descriptor. `url` alone can repeat across re-uploads of
 * identical bytes, so pair it with the digest.
 */
function descriptorKey(descriptor: BlobDescriptor): string {
  return `${descriptor.url}\u0000${descriptor.sha256 ?? ""}`;
}

/** The real attachments, in order, with in-flight placeholders dropped. */
export function compactImetaSlots(
  slots: (BlobDescriptor | null)[],
): BlobDescriptor[] {
  return slots.filter((d): d is BlobDescriptor => d !== null);
}

/**
 * Apply an updater written against the compacted list back onto `slots`.
 *
 * Replacing the array with the updater's result would renumber it while an
 * in-flight upload still holds an index from `reserveSlots`, so that upload's
 * `fillSlot` would overwrite an unrelated attachment. Instead:
 *
 * - survivors stay at the index they already occupy;
 * - removals become `null` rather than shifting their neighbours;
 * - genuinely new descriptors append after the reserved tail, where no pending
 *   `fillSlot` can reach them.
 *
 * An updater that returns its input unchanged (e.g. the snapshot-paste dedupe)
 * leaves `slots` exactly as it was, identity included.
 */
export function applyImetaUpdate(
  slots: (BlobDescriptor | null)[],
  update: (current: BlobDescriptor[]) => BlobDescriptor[],
): (BlobDescriptor | null)[] {
  const current = compactImetaSlots(slots);
  const next = update(current);
  if (next === current) return slots;

  const survivingKeys = new Set(next.map(descriptorKey));
  const preserved = slots.map((descriptor) =>
    descriptor === null || survivingKeys.has(descriptorKey(descriptor))
      ? descriptor
      : null,
  );
  const presentKeys = new Set(current.map(descriptorKey));
  const appended = next.filter(
    (descriptor) => !presentKeys.has(descriptorKey(descriptor)),
  );
  return appended.length > 0 ? [...preserved, ...appended] : preserved;
}
