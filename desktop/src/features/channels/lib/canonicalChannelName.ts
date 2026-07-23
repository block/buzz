/**
 * Returns the stored form of a channel name after removing the display prefix.
 * Keep this aligned with `buzz_core::channel::canonical_channel_name`.
 */
export function canonicalChannelName(name: string): string {
  return name.replace(/^[#\s]+/u, "").trimEnd();
}
