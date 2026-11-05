import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'pending_local_messages_provider.dart';

/// Delivery state of one of this session's outgoing messages.
enum LocalSendStatus { sending, sent, failed }

/// Outgoing messages this session signed in one channel, and which failed.
///
/// Whether a message is still in flight comes from
/// [pendingLocalMessagesProvider]: a relay echo or acknowledgement removes it
/// there, which also clears a failure whose acknowledgement was merely lost.
class LocalSendState {
  /// The most recently signed outgoing message, which carries the status line.
  final String? latestId;

  /// Messages whose publish was rejected, timed out, or had no connection.
  final Set<String> failedIds;

  /// Every message signed in this channel during this session.
  final Set<String> trackedIds;

  /// Creates a snapshot of outgoing delivery state.
  const LocalSendState({
    this.latestId,
    this.failedIds = const {},
    this.trackedIds = const {},
  });
}

/// Tracks outgoing delivery state for one channel.
class LocalSendStatusNotifier extends Notifier<LocalSendState> {
  /// The channel whose outgoing messages are tracked.
  final String channelId;

  /// Creates delivery state scoped to [channelId].
  LocalSendStatusNotifier(this.channelId);

  final _personalIds = <String>{};

  @override
  LocalSendState build() => const LocalSendState();

  /// Whether [eventId] was sent from Me's flat timeline, whose sender owns it.
  bool isPersonal(String eventId) => _personalIds.contains(eventId);

  /// Records a just-signed outgoing message as the latest one.
  void track(String eventId, {required bool personal}) {
    if (personal) _personalIds.add(eventId);
    state = LocalSendState(
      latestId: eventId,
      failedIds: state.failedIds,
      trackedIds: {...state.trackedIds, eventId},
    );
  }

  /// Marks [eventId] as failed so its bubble offers retry and delete.
  void fail(String eventId) {
    state = LocalSendState(
      latestId: state.latestId,
      failedIds: {...state.failedIds, eventId},
      trackedIds: state.trackedIds,
    );
  }

  /// Returns [eventId] to sending for a retry.
  void retrying(String eventId) {
    if (!state.failedIds.contains(eventId)) return;
    state = LocalSendState(
      latestId: state.latestId,
      failedIds: {...state.failedIds}..remove(eventId),
      trackedIds: state.trackedIds,
    );
  }

  /// Stops tracking a discarded message.
  void forget(String eventId) {
    _personalIds.remove(eventId);
    state = LocalSendState(
      latestId: state.latestId == eventId ? null : state.latestId,
      failedIds: {...state.failedIds}..remove(eventId),
      trackedIds: {...state.trackedIds}..remove(eventId),
    );
  }
}

/// Outgoing delivery state keyed by channel id.
final localSendStatusProvider =
    NotifierProvider.family<LocalSendStatusNotifier, LocalSendState, String>(
      LocalSendStatusNotifier.new,
    );

/// The status line to show for [eventId], or null when it shows none.
///
/// Failures always show. Sending and sent show only on the latest outgoing
/// message, like a messaging app's single "Delivered" line.
final localSendStatusForProvider = Provider.family
    .autoDispose<LocalSendStatus?, ({String channelId, String eventId})>((
      ref,
      args,
    ) {
      final send = ref.watch(localSendStatusProvider(args.channelId));
      if (!send.trackedIds.contains(args.eventId)) return null;
      final pending = ref.watch(
        pendingLocalMessagesProvider(
          args.channelId,
        ).select((events) => events.containsKey(args.eventId)),
      );
      if (pending && send.failedIds.contains(args.eventId)) {
        return LocalSendStatus.failed;
      }
      if (send.latestId != args.eventId) return null;
      return pending ? LocalSendStatus.sending : LocalSendStatus.sent;
    });
