import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';

/// Signed local auxiliary events whose publish has not yet been corroborated
/// by an authoritative relay EVENT or query result.
class PendingLocalAuxEventsNotifier extends Notifier<Map<String, NostrEvent>> {
  final String channelId;

  PendingLocalAuxEventsNotifier(this.channelId);

  @override
  Map<String, NostrEvent> build() => const {};

  void add(NostrEvent event) {
    state = {...state, event.id: event};
  }

  NostrEvent? take(String eventId) {
    final event = state[eventId];
    if (event == null) return null;
    final next = {...state}..remove(eventId);
    state = next;
    return event;
  }

  void confirm(Iterable<String> eventIds) {
    final confirmed = eventIds.toSet();
    if (!state.keys.any(confirmed.contains)) return;
    state = {
      for (final entry in state.entries)
        if (!confirmed.contains(entry.key)) entry.key: entry.value,
    };
  }
}

final pendingLocalAuxEventsProvider =
    NotifierProvider.family<
      PendingLocalAuxEventsNotifier,
      Map<String, NostrEvent>,
      String
    >(PendingLocalAuxEventsNotifier.new);
