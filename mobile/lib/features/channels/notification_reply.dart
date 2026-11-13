import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/auth/auth.dart';
import '../../shared/push/push_bridge.dart';
import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/drafts/compose_drafts_provider.dart';
import 'channel.dart';
import 'channel_management_provider.dart';
import 'channels_provider.dart';
import 'send_message_provider.dart';

/// Sends replies typed into notifications, and tells the notification
/// extension which community is selected. Listen to it from app start: iOS
/// may launch the app in the background just to send a reply.
///
/// See docs/adr/0004-notification-reply.md.
final notificationReplySenderProvider = Provider<void>((ref) {
  // Only a loaded selection is shared, so loading it at launch never
  // briefly clears it.
  ref.listen(activeCommunityProvider, (previous, next) {
    if (next case AsyncData(
      :final value,
    ) when previous?.value?.id != value?.id || previous is! AsyncData) {
      unawaited(shareSelectedCommunityWithNotifications(value?.id));
    }
  }, fireImmediately: true);

  var draining = false;
  var drainAgain = false;
  Future<void> drain() async {
    if (draining) {
      drainAgain = true;
      return;
    }
    draining = true;
    try {
      do {
        drainAgain = false;
        for (final reply in await takePendingBuzzNotificationReplies()) {
          try {
            final result = await sendNotificationReply(ref, reply);
            await completeBuzzNotificationReply(
              reply.id,
              sent: result.outcome == NotificationReplyOutcome.sent,
              unknown: result.outcome == NotificationReplyOutcome.unknown,
              draftThreadRootId: result.draftThreadRootId,
            );
          } catch (error) {
            // The native side gives up on it at its own deadline.
            debugPrint('[NotificationReply] not completed: $error');
          }
        }
      } while (drainAgain);
    } finally {
      draining = false;
    }
  }

  void onAvailable() => unawaited(drain());
  notificationReplyAvailable.addListener(onAvailable);
  ref.onDispose(() => notificationReplyAvailable.removeListener(onAvailable));
  unawaited(drain());
});

/// How long a publish waits for the relay's OK; see
/// [RelaySessionNotifier.publish].
const _publishTimeout = Duration(seconds: 8);

/// Time kept before the native deadline to report the outcome.
const _reportMargin = Duration(seconds: 2);

/// Longest wait for marking the conversation read after a reply is sent.
const _markReadDeadline = Duration(seconds: 3);

/// Whether a notification reply went out.
enum NotificationReplyOutcome {
  sent,

  /// Never published, or refused by the relay; its text is kept as a draft.
  notSent,

  /// Published, but no answer came from the relay, so it may have gone out.
  /// No draft is kept, so resending can't post it twice.
  unknown,
}

/// What happened to a notification reply, and the thread holding its draft
/// when one was kept.
typedef NotificationReplyResult = ({
  NotificationReplyOutcome outcome,
  String? draftThreadRootId,
});

/// Sends [reply] in a subthread under the notified message, mentioning its
/// sender while they're still in the conversation, then marks the
/// conversation read up to that message. A reply that wasn't published keeps
/// its text as a draft where it would have gone, in the reply's own community.
///
/// Everything happens before the reply's native deadline: publishing starts
/// only while there's still time to hear the relay's OK, so the outcome
/// reported is the real one.
@visibleForTesting
Future<NotificationReplyResult> sendNotificationReply(
  Ref ref,
  BuzzNotificationReply reply,
) async {
  // Sending ends early enough to mark read and report before the deadline.
  final left =
      reply.deadline.difference(DateTime.now()) -
      _reportMargin -
      _markReadDeadline;
  final publishWindow = left - _publishTimeout;
  if (publishWindow <= Duration.zero) {
    return (
      outcome: NotificationReplyOutcome.notSent,
      draftThreadRootId: await _keepDraft(ref, reply),
    );
  }
  // The background grace period would otherwise disconnect mid-reply.
  final release = ref
      .read(relaySessionProvider.notifier)
      .holdForBackgroundWork(left + _markReadDeadline);
  var tooLate = false;
  final publishBy = Timer(publishWindow, () => tooLate = true);
  var published = false;
  bool mayPublish() {
    if (tooLate) return false;
    published = true;
    return true;
  }

  try {
    try {
      await _send(
        ref,
        reply,
        connectTimeout: publishWindow,
        mayPublish: mayPublish,
      ).timeout(left);
    } catch (error) {
      debugPrint('[NotificationReply] not sent: $error');
      // Only the relay saying no settles a published reply as not sent.
      if (published && error is! RelayEventRejectedException) {
        return (
          outcome: NotificationReplyOutcome.unknown,
          draftThreadRootId: null,
        );
      }
      // A send still preparing must not publish after this.
      tooLate = true;
      return (
        outcome: NotificationReplyOutcome.notSent,
        draftThreadRootId: await _keepDraft(ref, reply),
      );
    } finally {
      publishBy.cancel();
    }
    try {
      final link = reply.link;
      final rootId = reply.replyRootId;
      final readState = ref.read(readStateProvider.notifier);
      readState.markContextRead(
        rootId == null ? link.channelId : threadContextKey(rootId),
        reply.messageCreatedAt,
      );
      // The app may be suspended right after the reply finishes.
      await readState.flush().timeout(_markReadDeadline);
    } catch (error) {
      debugPrint('[NotificationReply] not marked read: $error');
    }
    return (outcome: NotificationReplyOutcome.sent, draftThreadRootId: null);
  } finally {
    release();
  }
}

/// Publishes [reply] to its own community's relay, or throws. [mayPublish]
/// is asked right before the event goes out, after every wait, including a
/// rate limit's.
Future<void> _send(
  Ref ref,
  BuzzNotificationReply reply, {
  required Duration connectTimeout,
  required bool Function() mayPublish,
}) async {
  final link = reply.link;
  final auth = await ref.read(authProvider.future);
  final active = await ref.read(activeCommunityProvider.future);
  // Replying never switches communities, and only the selected one is
  // reachable.
  if (auth.status != AuthStatus.authenticated ||
      active?.id != link.communityId) {
    throw StateError('Reply is not for the selected community');
  }
  // Taken before any wait: its delivery check then refuses to publish if the
  // selected community changes.
  final send = ref.read(sendMessageProvider);
  final channels = await ref.read(channelsProvider.future);
  final channel = channels
      .where((channel) => channel.id == link.channelId)
      .firstOrNull;
  if (channel == null || !channel.isMember) {
    throw StateError('Not a member of the reply channel');
  }
  final connected = await ref
      .read(relaySessionProvider.notifier)
      .connectForBackgroundWork(timeout: connectTimeout);
  if (!connected) throw StateError('Relay not reachable');
  if (ref.read(activeCommunityProvider).value?.id != link.communityId) {
    throw StateError('Selected community changed');
  }
  // Only a current member can be mentioned; a sender who has left still gets
  // the reply in the conversation.
  final sender = reply.senderPubkey.toLowerCase();
  final members = await ref.read(channelMembersProvider(link.channelId).future);
  final senderIsMember = members.any(
    (member) => member.pubkey.toLowerCase() == sender,
  );
  final head = _replyHead(reply, channel);
  await send(
    channelId: link.channelId,
    content: reply.text,
    parentEventId: head == null ? null : link.messageId,
    rootEventId: head,
    mentionPubkeys: senderIsMember ? [reply.senderPubkey] : const [],
    channel: channel,
    mayPublish: mayPublish,
  );
}

/// The thread [reply] goes in: the notified message's thread, or a new
/// subthread under it. A Solo chat takes no thread replies, so null there.
String? _replyHead(BuzzNotificationReply reply, Channel channel) =>
    channel.isSolo ? null : reply.replyRootId ?? reply.link.messageId;

/// Keeps the reply's text as a draft, after any draft already there, in the
/// reply's own community, and returns the thread it's in once it's saved. It
/// goes where the reply would have gone: the subthread under the notified
/// message, or a Solo chat itself. Whether it's a Solo chat comes from the
/// loaded channel, or else from the notification.
Future<String?> _keepDraft(Ref ref, BuzzNotificationReply reply) async {
  final link = reply.link;
  final channel =
      ref.read(activeCommunityProvider).value?.id == link.communityId
      ? ref
            .read(channelsProvider)
            .value
            ?.where((channel) => channel.id == link.channelId)
            .firstOrNull
      : null;
  final solo = channel?.isSolo ?? reply.inSolo;
  final threadHeadId = solo ? null : link.messageId;
  try {
    final communities = await ref.read(communityListProvider.future);
    final community = communities
        .where((community) => community.id == link.communityId)
        .firstOrNull;
    if (community == null) {
      throw StateError('Community ${link.communityId} not found');
    }
    await ref
        .read(composeDraftsProvider.notifier)
        .append(
          config: RelayConfig(
            baseUrl: community.relayUrl,
            nsec: community.nsec,
          ),
          key: composeDraftKey(link.channelId, threadHeadId: threadHeadId),
          channelId: link.channelId,
          threadHeadId: threadHeadId,
          text: reply.text,
        );
  } catch (error) {
    debugPrint('[NotificationReply] draft not kept: $error');
  }
  return threadHeadId;
}
