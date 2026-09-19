import 'package:hooks_riverpod/hooks_riverpod.dart';

const _localMessageSendAnimationClaimWindow = Duration(seconds: 1);

/// Recently signed local messages that may play the send entrance animation.
///
/// The short claim window prevents a message first rendered much later (for
/// example after opening its channel) from looking newly sent.
class LocalMessageSendAnimationNotifier
    extends Notifier<Map<String, DateTime>> {
  /// The channel whose outgoing messages are tracked by this notifier.
  final String channelId;

  /// Creates animation state scoped to [channelId].
  LocalMessageSendAnimationNotifier(this.channelId);

  @override
  Map<String, DateTime> build() => const {};

  /// Marks [eventId] as a newly signed local message eligible for animation.
  void mark(String eventId) {
    final now = DateTime.now();
    state = {
      for (final entry in state.entries)
        if (now.difference(entry.value) <=
            _localMessageSendAnimationClaimWindow)
          entry.key: entry.value,
      eventId: now,
    };
  }
}

/// Whether [eventId] was marked recently enough to animate as a local send.
bool isRecentLocalMessageSendAnimation(
  Map<String, DateTime> animations,
  String eventId, {
  DateTime? now,
}) {
  final markedAt = animations[eventId];
  if (markedAt == null) return false;
  return (now ?? DateTime.now()).difference(markedAt) <=
      _localMessageSendAnimationClaimWindow;
}

/// Recent local-send animation claims keyed by channel and event ID.
final localMessageSendAnimationProvider =
    NotifierProvider.family<
      LocalMessageSendAnimationNotifier,
      Map<String, DateTime>,
      String
    >(LocalMessageSendAnimationNotifier.new);
