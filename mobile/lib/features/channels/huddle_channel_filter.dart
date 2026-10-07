import 'dart:convert';

import '../../shared/relay/relay.dart';

/// Returns backing-channel IDs whose Huddle starts are signed by the
/// authoritative channel owner.
Set<String> huddleBackingChannelIds(
  Iterable<NostrEvent> starts,
  Iterable<NostrEvent> membershipSnapshots,
) {
  final ownersByChannelId = <String, Set<String>>{};
  for (final snapshot in membershipSnapshots) {
    if (snapshot.kind != 39002) continue;
    final channelId = snapshot.getTagValue('d');
    if (channelId == null) continue;
    ownersByChannelId[channelId] = {
      for (final member in membersFromEvent(snapshot))
        if (member.role == 'owner') member.pubkey.toLowerCase(),
    };
  }

  final backingIds = <String>{};
  for (final start in starts) {
    final backingChannelId = _huddleBackingChannelId(start.content);
    if (backingChannelId != null &&
        (ownersByChannelId[backingChannelId] ?? const {}).contains(
          start.pubkey.toLowerCase(),
        )) {
      backingIds.add(backingChannelId);
    }
  }
  return backingIds;
}

String? _huddleBackingChannelId(String content) {
  try {
    final decoded = jsonDecode(content);
    if (decoded is! Map) return null;
    final value = decoded['ephemeral_channel_id'];
    if (value is! String || value.isEmpty) return null;
    return value;
  } catch (_) {
    return null;
  }
}
