import '../../../shared/relay/nostr_models.dart';

bool shouldNotifyForEvent(
  NostrEvent event,
  String myPubkey, {
  Set<String> participatedRootIds = const {},
  Set<String> authoredRootIds = const {},
}) {
  if (!EventKind.channelMessageEventKinds.contains(event.kind)) return false;

  if (event.pubkey.toLowerCase() == myPubkey.toLowerCase()) return false;

  final ref = event.threadReference;
  if (ref.parentId == null) return true;

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

  final rootId = ref.rootId;
  return rootId != null &&
      (participatedRootIds.contains(rootId) ||
          authoredRootIds.contains(rootId));
}
