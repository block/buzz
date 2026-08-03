import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';

enum LocalMessageDeliveryState { sending, unconfirmed, sent, failed }

/// Delivery state for locally signed messages. This is deliberately separate
/// from the event map so a confirmed message can remain visibly `sent` while a
/// failed message retains its original signed event for idempotent retry.
class LocalMessageDeliveryStatesNotifier
    extends Notifier<Map<String, LocalMessageDeliveryState>> {
  final String channelId;

  LocalMessageDeliveryStatesNotifier(this.channelId);

  @override
  Map<String, LocalMessageDeliveryState> build() => const {};

  void markSending(String eventId) =>
      _set(eventId, LocalMessageDeliveryState.sending);

  void markSent(String eventId) =>
      _set(eventId, LocalMessageDeliveryState.sent);

  void markUnconfirmed(String eventId) =>
      _set(eventId, LocalMessageDeliveryState.unconfirmed);

  void markFailed(String eventId) =>
      _set(eventId, LocalMessageDeliveryState.failed);

  void _set(String eventId, LocalMessageDeliveryState deliveryState) {
    state = {...state, eventId: deliveryState};
  }
}

final localMessageDeliveryStatesProvider =
    NotifierProvider.family<
      LocalMessageDeliveryStatesNotifier,
      Map<String, LocalMessageDeliveryState>,
      String
    >(LocalMessageDeliveryStatesNotifier.new);

/// Signed local messages whose publish has not yet been corroborated by an
/// authoritative relay EVENT or query result.
class PendingLocalMessagesNotifier extends Notifier<Map<String, NostrEvent>> {
  final String channelId;

  PendingLocalMessagesNotifier(this.channelId);

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

final pendingLocalMessagesProvider =
    NotifierProvider.family<
      PendingLocalMessagesNotifier,
      Map<String, NostrEvent>,
      String
    >(PendingLocalMessagesNotifier.new);
