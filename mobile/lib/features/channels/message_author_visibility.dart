import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'channel_management_provider.dart';
import 'channels_provider.dart';

/// Keep names unless membership confirms a conversation with only two people.
bool showMessageAuthorName(
  WidgetRef ref,
  String channelId,
  String authorPubkey,
  String? viewerPubkey,
) {
  final channel = ref.watch(
    channelsProvider.select(
      (value) =>
          value.value?.where((channel) => channel.id == channelId).firstOrNull,
    ),
  );
  if (channel == null) return true;
  final members = ref.watch(channelMembersProvider(channelId)).value;
  // The DM header still uses metadata. Keep attribution while that snapshot
  // disagrees with current membership, even if only one counterpart remains.
  if (channel.isDm && members != null) {
    final metadata = channel.participantPubkeys
        .map((key) => key.trim().toLowerCase())
        .where((key) => key.isNotEmpty)
        .toSet();
    final current = members
        .map((member) => member.pubkey.trim().toLowerCase())
        .where((key) => key.isNotEmpty)
        .toSet();
    if (metadata.length != current.length || !metadata.containsAll(current)) {
      return true;
    }
  }
  final participants =
      (members?.map((member) => member.pubkey) ??
              (channel.isDm ? channel.participantPubkeys : const <String>[]))
          .map((pubkey) => pubkey.trim().toLowerCase())
          .where((pubkey) => pubkey.isNotEmpty)
          .toSet();
  return participants.length != 2 ||
      !participants.contains(viewerPubkey?.toLowerCase()) ||
      !participants.contains(authorPubkey.toLowerCase());
}
