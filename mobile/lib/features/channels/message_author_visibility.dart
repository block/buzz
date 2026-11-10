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
  if (channel == null || (!channel.isDm && !channel.isSolo)) return true;
  final members = ref.watch(channelMembersProvider(channelId)).value;
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
