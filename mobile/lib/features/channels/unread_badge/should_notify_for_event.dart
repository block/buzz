import '../../../shared/mentions/mention_tags.dart';
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

  if (tagsAddress(event.tags, myPubkey)) return true;

  final eventChannelId = channelId ?? event.channelId;
  if (eventChannelId != null && mutedChannelIds.contains(eventChannelId)) {
    return false;
  }

  if (ref.parentId == null) return true;

  final rootId = ref.rootId;
  if (rootId != null && mutedRootIds.contains(rootId)) {
    return false;
  }

  return isInViewerThread(
    rootId,
    participatedRootIds: participatedRootIds,
    followedRootIds: followedRootIds,
    authoredRootIds: authoredRootIds,
  );
}

/// Whether [rootId] is one of the viewer's threads: they replied in it,
/// follow it, or started it.
bool isInViewerThread(
  String? rootId, {
  required Set<String> participatedRootIds,
  required Set<String> followedRootIds,
  required Set<String> authoredRootIds,
}) =>
    rootId != null &&
    (participatedRootIds.contains(rootId) ||
        followedRootIds.contains(rootId) ||
        authoredRootIds.contains(rootId));
