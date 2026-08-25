import { CircleDot, FileText, Hash, Lock } from "lucide-react";
import { registerChannelFeature } from "./registry";

/**
 * The four channel-classification cases `ChatHeader`'s `ChannelIcon` used to
 * check inline (dm → private → forum → hash), now expressed as
 * priority-ordered channel-feature plugins. Registering them here — rather
 * than inlining the checks at each call site — gives `ChatHeader` and
 * `ChannelScreen` (which only needs the `forum` case, for its content
 * dispatch) one shared source of truth for "what kind of channel is this",
 * instead of two places independently re-deriving the same cascade.
 *
 * `stream` is an explicit catch-all (rather than leaving plain channels
 * unclassified) so `classifyChannel` always returns a match once these are
 * registered, matching `ChannelIcon`'s original `Hash`-by-default fallthrough.
 */
export function registerBuiltinChannelFeatures(): void {
  registerChannelFeature({
    id: "dm",
    priority: 0,
    glyph: CircleDot,
    parseBinding: (channel) => (channel.channelType === "dm" ? true : null),
  });
  registerChannelFeature({
    id: "private-channel",
    priority: 10,
    glyph: Lock,
    parseBinding: (channel) => (channel.visibility === "private" ? true : null),
  });
  registerChannelFeature({
    id: "forum",
    priority: 20,
    glyph: FileText,
    parseBinding: (channel) => (channel.channelType === "forum" ? true : null),
  });
  registerChannelFeature({
    id: "stream",
    priority: 30,
    glyph: Hash,
    parseBinding: () => true,
  });
}
