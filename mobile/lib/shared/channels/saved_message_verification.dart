import 'dart:convert';

import '../crypto/event_signing.dart';
import '../relay/nostr_models.dart';

/// Kinds only the relay signs that the saved message store may hold. Each is
/// trusted only when signed by the relay's NIP-11 `self` key.
///
/// Mixed-author kinds stay out: a Huddle end (48103) may come from its
/// creator, and workflow or moderation messages are ordinary kind:9 events
/// signed by the relay.
const _relayAuthoredKinds = {
  39000, // NIP-29 group metadata
  39002, // NIP-29 group members
  EventKind.channelThreadSummary, // 39005
  EventKind.channelWindowBounds, // 39006
  EventKind.systemMessage, // 40099
  EventKind.huddleParticipantJoined, // 48101
  EventKind.huddleParticipantLeft, // 48102
};

final _lowerHex64 = RegExp(r'^[0-9a-f]{64}$');

/// Checks the current relay binding for an event whose proof is already known.
bool matchesSavedMessageAuthority(NostrEvent event, String? relayAuthor) =>
    !_relayAuthoredKinds.contains(event.kind) ||
    (relayAuthor != null &&
        _lowerHex64.hasMatch(relayAuthor) &&
        event.pubkey == relayAuthor);

/// Whether [event] may be saved: its id and BIP-340 signature are valid,
/// and a relay-authored kind is signed by [relayAuthor], the relay's NIP-11
/// `self` key.
///
/// With [relayAuthor] unknown or malformed, relay-authored kinds are refused
/// and every other kind still passes on its own proof.
bool verifySavedMessageEvent(NostrEvent event, {required String? relayAuthor}) {
  if (!matchesSavedMessageAuthority(event, relayAuthor)) return false;
  try {
    verifiedNostrEventFromJson(jsonEncode(event.toJson()));
    return true;
  } catch (_) {
    return false;
  }
}
