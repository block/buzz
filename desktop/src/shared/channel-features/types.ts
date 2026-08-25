import type { LucideIcon } from "lucide-react";
import type { Channel } from "@/shared/api/types";

/**
 * The minimal channel shape a plugin's `parseBinding` needs to classify a
 * channel. Kept narrower than the full `Channel` so call sites that only
 * have a partial channel on hand (e.g. `ChatHeader`, which receives
 * `channelType`/`visibility` as separate props rather than a `Channel`) can
 * classify without constructing one.
 */
export type ChannelClassifyInput = Pick<Channel, "channelType" | "visibility">;

/**
 * The result of classifying a channel: which plugin matched, and the value
 * its `parseBinding` returned. `T` defaults to `unknown` so call sites that
 * don't care about the specific plugin's binding shape (e.g. glyph
 * resolution) can use the bare `ChannelBinding` alias.
 */
export interface ChannelBinding<T = unknown> {
  pluginId: string;
  value: T;
}

/**
 * A channel-feature plugin: classifies a channel into a typed binding and
 * supplies the glyph shown for channels it matches. Modeled on the existing
 * `shared/features/` flag manifest's "typed definition + resolver + gate"
 * ergonomics, but for channel-binding plugins instead of preview flags.
 *
 * This is the seed of the plugin surface proposed in
 * https://github.com/block/buzz/issues/3280: today it unifies channel
 * classification for the header glyph and the channel-screen content
 * dispatch (see `ChatHeader.tsx` / `ChannelScreen.tsx`). A plugin that wants
 * to contribute its own tab bar, settings section, or sidebar affordance —
 * e.g. hosting an MCP App as a channel tab (block/buzz#3275) — is a natural
 * extension of `T` and this interface once a concrete second consumer shows
 * up; see PR_DESCRIPTION.md's Follow-ups for the shape that would take.
 */
export interface ChannelFeaturePlugin<T> {
  /** Unique id — duplicate registration is a no-op (warn + ignore). */
  id: string;
  /**
   * Classify `channel` into this plugin's binding shape, or `null` if it
   * doesn't match.
   */
  parseBinding: (channel: ChannelClassifyInput) => T | null;
  /** Glyph shown for channels this plugin matches (header/sidebar/intro). */
  glyph?: LucideIcon;
  /** Lower runs first when classifying; ties keep registration order. Default 0. */
  priority?: number;
}
