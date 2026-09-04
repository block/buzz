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

// The resurface subscription is `#h`-scoped to the hidden-DM set, so the relay
// only delivers events already addressed to a hidden channel the reader belongs
// to. Eligibility therefore drops the `#p` requirement — an untagged DM (a CLI
// or agent send that omits participant `p` tags) still resurfaces the row.
bool isIncomingChannelMessageFromOther(NostrEvent event, String currentPubkey) {
  final self = currentPubkey.trim().toLowerCase();
  return event.channelId != null &&
      EventKind.channelMessageEventKinds.contains(event.kind) &&
      event.pubkey.toLowerCase() != self;
}
