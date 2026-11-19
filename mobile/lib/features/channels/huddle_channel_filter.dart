import 'dart:convert';

import '../../shared/relay/relay.dart';

/// Desktop's presentation-only fallback for generated private Huddle rooms.
/// This never establishes a parent relationship or grants conversation access.
bool isGeneratedHuddleChannel({
  required String id,
  required String name,
  required String channelType,
  required String visibility,
  required int? ttlSeconds,
}) =>
    id.length >= 8 &&
    channelType == 'stream' &&
    visibility == 'private' &&
    ttlSeconds != null &&
    ttlSeconds > 0 &&
    ttlSeconds <= 2147483647 &&
    name == 'huddle-${id.substring(0, 8)}';

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

/// Desktop's stronger classification: an immutable creator-matched creation
/// receipt, independent of the room's current name or editable description.
/// Presentation only; never grants access to a parent or backing conversation.
Set<String> createdHuddleChannelIds(
  Iterable<NostrEvent> creations,
  Iterable<ChannelData> metadata,
) {
  final byId = {for (final channel in metadata) channel.id: channel};
  final parentMarker = RegExp(
    r'^Buzz Huddle \(buzz\.huddles/v1\)\nparent:[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$',
  );
  final pubkeyPattern = RegExp(r'^[0-9a-f]{64}$');
  final ids = <String>{};
  for (final event in creations) {
    if (event.kind != 9007) continue;
    final coordinates = event.tags
        .where((tag) => tag.isNotEmpty && tag[0] == 'h')
        .toList();
    if (coordinates.length != 1 || coordinates.single.length != 2) continue;
    final channel = byId[coordinates.single[1]];
    if (channel == null || !pubkeyPattern.hasMatch(channel.creator ?? '')) {
      continue;
    }
    if (event.pubkey == channel.creator &&
        event.getTagValue('visibility') == 'private' &&
        event.getTagValue('channel_type') == 'stream' &&
        parentMarker.hasMatch(event.getTagValue('about') ?? '')) {
      ids.add(channel.id);
    }
  }
  return ids;
}
