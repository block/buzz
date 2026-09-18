import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/relay.dart';

/// Archived identity pubkeys from the relay's NIP-IA snapshot (kind:13535).
///
/// Mirrors desktop's `useArchivedIdentitiesQuery`: archived identities stay
/// renderable in history but are hidden from forward-looking discovery
/// surfaces such as mention autocomplete. Fail-open — the set is empty while
/// the snapshot loads, so a cold start can't briefly hide everyone.
///
/// Watches the session and only fetches after the WebSocket connects.
final archivedIdentitiesProvider = FutureProvider<Set<String>>((ref) async {
  final sessionState = ref.watch(relaySessionProvider);
  if (sessionState.status != SessionStatus.connected) return const {};
  final session = ref.read(relaySessionProvider.notifier);
  final events = await session.fetchHistory(NostrFilters.archivedIdentities());
  if (events.isEmpty) return const {};
  return {
    for (final tag in events.first.tags)
      if (tag.length >= 2 && tag[0] == 'p') tag[1].toLowerCase(),
  };
});
