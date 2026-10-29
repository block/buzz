import 'dart:convert';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../../shared/relay/relay.dart';

/// Modern desktop agents advertise bot status in their kind-0 profile.
/// Keep this directory separate from legacy mention-routing metadata.
final communityBotProfilesProvider = FutureProvider<List<String>>((ref) async {
  final state = ref.watch(relaySessionProvider);
  if (state.status != SessionStatus.connected) return const [];
  final events = await ref
      .read(relaySessionProvider.notifier)
      .fetchHistory(const NostrFilter(kinds: [0], limit: 100));
  return [
    for (final event in events)
      if (isCommunityBotProfile(event)) event.pubkey,
  ];
});

bool isCommunityBotProfile(NostrEvent event) {
  if (event.kind != 0) return false;
  try {
    final profile = jsonDecode(event.content);
    return profile is Map<String, dynamic> && profile['bot'] == true;
  } catch (_) {
    return false;
  }
}
