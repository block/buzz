import '../../shared/relay/relay.dart';

final _hexPubkey = RegExp(r'^[0-9a-f]{64}$');

Set<String> dmPeerPubkeysFromMembers(
  Iterable<String> memberPubkeys,
  String currentPubkey,
) {
  final self = currentPubkey.trim().toLowerCase();
  final members = memberPubkeys
      .map((pubkey) => pubkey.trim().toLowerCase())
      .where(_hexPubkey.hasMatch)
      .toSet();
  if (!_hexPubkey.hasMatch(self) || !members.contains(self)) return {};
  return members..remove(self);
}

bool isIncomingDmMessageEvent(NostrEvent event, String currentPubkey) {
  final self = currentPubkey.trim().toLowerCase();
  return event.channelId != null &&
      EventKind.channelMessageEventKinds.contains(event.kind) &&
      event.pubkey.toLowerCase() != self &&
      event.tags.any(
        (tag) =>
            tag.length >= 2 && tag[0] == 'p' && tag[1].toLowerCase() == self,
      );
}
