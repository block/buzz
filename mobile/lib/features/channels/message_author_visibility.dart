import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'channel_management_provider.dart';
import 'channels_provider.dart';

Set<String> _normalizedKeys(Iterable<String> keys) => keys
    .map((key) => key.trim().toLowerCase())
    .where((key) => key.isNotEmpty)
    .toSet();

/// The two lowercase pubkeys of a conversation whose membership confirms
/// exactly two people, or null otherwise.
///
/// Computed once per channel so message rows do not each scan the roster.
/// The record compares by value, so rows rebuild only when the pair changes,
/// not on every channel or roster emission.
final confirmedConversationPairProvider = Provider.autoDispose
    .family<(String, String)?, String>((ref, channelId) {
      final channel = ref.watch(
        channelsProvider.select(
          (value) => value.value
              ?.where((channel) => channel.id == channelId)
              .firstOrNull,
        ),
      );
      if (channel == null) return null;
      final members = watchLatestChannelMembers(ref, channelId);
      final current = members == null
          ? null
          : _normalizedKeys(members.map((member) => member.pubkey));
      if (channel.isDm && current != null) {
        // The DM header still uses metadata. Keep attribution while that
        // snapshot disagrees with current membership, even if only one
        // counterpart remains.
        final metadata = _normalizedKeys(channel.participantPubkeys);
        if (metadata.length != current.length ||
            !metadata.containsAll(current)) {
          return null;
        }
      }
      final participants =
          current ??
          (channel.isDm
              ? _normalizedKeys(channel.participantPubkeys)
              : const <String>{});
      if (participants.length != 2) return null;
      final sorted = participants.toList()..sort();
      return (sorted[0], sorted[1]);
    });

/// Keep names unless membership confirms a conversation with only two people.
bool showMessageAuthorName(
  WidgetRef ref,
  String channelId,
  String authorPubkey,
  String? viewerPubkey,
) {
  final pair = ref.watch(confirmedConversationPairProvider(channelId));
  if (pair == null) return true;
  bool inPair(String? pubkey) {
    final key = pubkey?.toLowerCase();
    return key == pair.$1 || key == pair.$2;
  }

  return !inPair(viewerPubkey) || !inPair(authorPubkey);
}
