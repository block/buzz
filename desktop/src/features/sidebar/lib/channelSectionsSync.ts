import { KIND_CHANNEL_SECTIONS } from "@/shared/constants/kinds";
import {
  clearChannelSectionsOutbox,
  parseChannelSectionPayload,
  writeChannelSectionsOutbox,
  type ChannelSection,
  type ChannelSectionStore,
} from "./channelSectionsStorage";
import { WholeBlobSyncManager, type RemoteBlob } from "./wholeBlobSyncManager";

export type RemoteSections = RemoteBlob<ChannelSectionStore>;

function sectionsStoresEqual(
  a: ChannelSectionStore,
  b: ChannelSectionStore,
): boolean {
  if (a.sections.length !== b.sections.length) return false;
  for (let i = 0; i < a.sections.length; i++) {
    const as = a.sections[i] as ChannelSection | undefined;
    const bs = b.sections[i] as ChannelSection | undefined;
    if (
      !as ||
      !bs ||
      as.id !== bs.id ||
      as.name !== bs.name ||
      as.icon !== bs.icon ||
      as.order !== bs.order
    )
      return false;
  }
  const aKeys = Object.keys(a.assignments);
  const bKeys = Object.keys(b.assignments);
  if (aKeys.length !== bKeys.length) return false;
  for (const key of aKeys) {
    if (a.assignments[key] !== b.assignments[key]) return false;
  }
  return true;
}

export class ChannelSectionSyncManager extends WholeBlobSyncManager<ChannelSectionStore> {
  constructor(pubkey: string, relayUrl: string) {
    super(pubkey, relayUrl, {
      kind: KIND_CHANNEL_SECTIONS,
      dTag: "channel-sections",
      logPrefix: "channelSectionsSync",
      parse: parseChannelSectionPayload,
      serializePayload: (store) => ({
        version: 1,
        sections: store.sections,
        assignments: store.assignments,
      }),
      writeOutbox: writeChannelSectionsOutbox,
      clearOutbox: clearChannelSectionsOutbox,
      storesEqual: sectionsStoresEqual,
      isLocalNonEmpty: (s) => s.sections.length > 0,
    });
  }

  /** Publish a sections store, debounced to 2s. Returns whether the intent is durably held. */
  publishSections(
    store: ChannelSectionStore,
    isRestoredReplay = false,
    restoredQueuedAt?: number,
  ): boolean {
    return this.publish(store, isRestoredReplay, restoredQueuedAt);
  }

  /** Fetch the current remote head for this pubkey's sections blob. */
  fetchRemoteSections() {
    return this.fetchRemoteBlob();
  }

  /** Subscribe to live relay events for this pubkey's sections blob. */
  subscribeToSections(onUpdate: (remote: RemoteSections) => void) {
    return this.subscribeLive(onUpdate);
  }
}
