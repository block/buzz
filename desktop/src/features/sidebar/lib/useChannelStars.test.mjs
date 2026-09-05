import {
  MAX_CHANNEL_STAR_ENTRIES,
  readChannelStarsStore,
  storageKey,
} from "./channelStarsStorage.ts";
import { useChannelStars } from "./useChannelStars.ts";
import { runMergeLaneHookSuite } from "./mergeLaneHook.shared.test.mjs";
import { runMergeLaneCarlSuite } from "./mergeLaneSyncCarl.shared.test.mjs";

runMergeLaneHookSuite({
  label: "stars",
  entryValueField: "starred",
  idsField: "starredChannelIds",
  trueAction: "starChannel",
  falseAction: "unstarChannel",
  dTag: "channel-stars",
  outboxKeyPrefix: "buzz-channel-stars-outbox.v1",
  MAX_ENTRIES: MAX_CHANNEL_STAR_ENTRIES,
  readStore: readChannelStarsStore,
  storageKey,
  useHook: useChannelStars,
  makePayload: (channels) => JSON.stringify({ version: 1, channels }),
});

runMergeLaneCarlSuite({
  label: "stars",
  MAX_ENTRIES: MAX_CHANNEL_STAR_ENTRIES,
  entryValueField: "starred",
  trueAction: "starChannel",
  dTag: "channel-stars",
  outboxKeyPrefix: "buzz-channel-stars-outbox.v1",
  readStore: readChannelStarsStore,
  storageKey,
  useHook: useChannelStars,
  makePayload: (channels) => JSON.stringify({ version: 1, channels }),
});
