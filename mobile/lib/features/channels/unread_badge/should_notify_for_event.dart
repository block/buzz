import '../../../shared/relay/nostr_models.dart';

bool shouldNotifyForEvent(
  NostrEvent event,
  String myPubkey, {
  Set<String> participatedRootIds = const {},
  Set<String> followedRootIds = const {},
  Set<String> authoredRootIds = const {},
  Set<String> mutedRootIds = const {},
  Set<String> mutedChannelIds = const {},
  String? channelId,
}) {
  if (!EventKind.channelMessageEventKinds.contains(event.kind)) return false;

  if (event.pubkey.toLowerCase() == myPubkey.toLowerCase()) return false;

  final ref = event.threadReference;

  for (final tag in event.tags) {
    if (tag.length >= 2 && tag[0] == 'broadcast' && tag[1] == '1') {
      return true;
    }
  }

  final normalizedPk = myPubkey.toLowerCase();
  for (final tag in event.tags) {
    if (tag.length >= 2 &&
        tag[0] == 'p' &&
        tag[1].toLowerCase() == normalizedPk) {
      return true;
    }
  }

  final eventChannelId = channelId ?? event.channelId;
  if (eventChannelId != null && mutedChannelIds.contains(eventChannelId)) {
    return false;
  }

  if (ref.parentId == null) return true;

  final rootId = ref.rootId;
  if (rootId != null && mutedRootIds.contains(rootId)) {
    return false;
  }

  return rootId != null &&
      (participatedRootIds.contains(rootId) ||
          followedRootIds.contains(rootId) ||
          authoredRootIds.contains(rootId));
}
