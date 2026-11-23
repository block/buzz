import 'dart:async';

import 'package:clock/clock.dart';
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
              draftKept: result.outcome != NotificationReplyOutcome.lost,
            );
          } catch (error) {
            // Unreported, it stays journaled: iOS gives up on it at its own
            // deadline, and a later launch keeps its draft.
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

  /// Like [notSent], but its community is gone, or is now signed in as
  /// another account, so no draft is kept. Its "Reply not sent" notification
  /// is the only place its text stays.
  lost,

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
  // An earlier launch was given it: it may have gone out, so it is never sent
  // again, and is kept as a draft only when it surely didn't.
  if (reply.recovered) {
    if (reply.mayHavePublished) {
      return (
        outcome: NotificationReplyOutcome.unknown,
        draftThreadRootId: null,
      );
    }
    return _notSent(ref, reply);
  }
  // Publishing starts early enough to hear the OK, mark read and report
  // before the deadline. The cutoff comes from the deadline itself, so a
  // suspension between clock readings can't move it later.
  final publishDeadline = reply.deadline.subtract(
    _publishTimeout + _markReadDeadline + _reportMargin,
  );
  final publishWindow = publishDeadline.difference(clock.now());
  final left = publishWindow + _publishTimeout;
  if (publishWindow <= Duration.zero) {
    return _notSent(ref, reply);
  }
  // The background grace period would otherwise disconnect mid-reply.
  final release = ref
      .read(relaySessionProvider.notifier)
      .holdForBackgroundWork(left + _markReadDeadline);
  var tooLate = false;
  final publishBy = Timer(publishWindow, () => tooLate = true);
  // The clock too: after a suspension, the timer may not have run yet.
  bool late() => tooLate || !clock.now().isBefore(publishDeadline);
  var published = false;
  // Asked right before the event goes out. The reply is saved as maybe
  // published first, so a relaunch never resends it; iOS refuses once it has
  // already ended the reply as not sent.
  Future<bool> mayPublish() async {
    if (late()) return false;
    if (!await markBuzzNotificationReplyPublishing(reply.id)) {
      tooLate = true;
      return false;
    }
    if (late()) return false;
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
      return await _notSent(ref, reply);
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
  required Future<bool> Function() mayPublish,
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
  if (!_isReplyAccount(reply, active!)) {
    throw StateError('Reply community is signed in as another account');
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
  final current = ref.read(activeCommunityProvider).value;
  if (current?.id != link.communityId || !_isReplyAccount(reply, current!)) {
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

/// Whether [community] is still signed in as the account [reply] was typed
/// for: the key it signs with. A notification from an earlier build doesn't
/// say, so its reply is checked only by its community, as before.
bool _isReplyAccount(BuzzNotificationReply reply, Community community) {
  final recipient = reply.recipientPubkey;
  final account = pubkeyFromNsec(community.nsec?.trim()) ?? community.pubkey;
  return recipient == null || account?.toLowerCase() == recipient.toLowerCase();
}

/// The result for a reply that wasn't published: [NotificationReplyOutcome.notSent]
/// with its draft's thread, or [NotificationReplyOutcome.lost] when its
/// community is gone or signed in as another account.
Future<NotificationReplyResult> _notSent(
  Ref ref,
  BuzzNotificationReply reply,
) async {
  final draft = await _keepDraft(ref, reply);
  return draft == null
      ? (outcome: NotificationReplyOutcome.lost, draftThreadRootId: null)
      : (
          outcome: NotificationReplyOutcome.notSent,
          draftThreadRootId: draft.threadRootId,
        );
}

/// Keeps the reply's text as a draft, after any draft already there, in the
/// reply's own community, and returns the thread it's in once it's saved, or
/// throws: the reply then stays with iOS until a later launch saves it, as it
/// does while this build hides that community. When that community is gone,
/// or is signed in as another account, nothing is kept and this returns
/// null. It
/// goes where the reply would have gone: the subthread under the notified
/// message, or a Solo chat itself. Whether it's a Solo chat comes from the
/// loaded channel, or else from the notification.
Future<({String? threadRootId})?> _keepDraft(
  Ref ref,
  BuzzNotificationReply reply,
) async {
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
  final communities = await ref.read(communityListProvider.future);
  final community = communities
      .where((community) => community.id == link.communityId)
      .firstOrNull;
  if (community == null) {
    // Hidden by this build but still saved: kept with iOS for a build that
    // shows it.
    final id = link.communityId;
    if (id != null && await ref.read(communityStorageProvider).keeps(id)) {
      throw StateError('Reply community hidden in this build');
    }
    // Removed, or signed out: there's nowhere to keep it, and keeping it with
    // iOS would retry it on every launch.
    return null;
  }
  // Another account's drafts are no place for it.
  if (!_isReplyAccount(reply, community)) return null;
  await ref
      .read(composeDraftsProvider.notifier)
      .append(
        config: RelayConfig(baseUrl: community.relayUrl, nsec: community.nsec),
        key: composeDraftKey(link.channelId, threadHeadId: threadHeadId),
        channelId: link.channelId,
        threadHeadId: threadHeadId,
        text: reply.text,
      );
  return (threadRootId: threadHeadId);
}
