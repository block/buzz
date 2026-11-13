import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/auth/auth.dart';
import '../../shared/push/push_bridge.dart';
import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import '../../shared/relay/relay.dart';
import '../activity/compose_drafts_provider.dart';
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
            final sent = await sendNotificationReply(ref, reply);
            await completeBuzzNotificationReply(reply.id, sent: sent);
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

/// How long a reply may take to get ready to publish.
const _prepareDeadline = Duration(seconds: 12);

/// How long a reply may take to send, including getting ready. iOS gives the
/// app about 30 s and the native side gives up at 25 s.
const _sendDeadline = Duration(seconds: 20);

/// How long marking the conversation read may take after a reply is sent.
const _markReadDeadline = Duration(seconds: 3);

/// Sends [reply] in a subthread under the notified message, mentioning its
/// sender, then marks the conversation read up to that message. Returns
/// whether it was sent; an unsent reply's text is kept as a draft where its
/// notification opens, in the reply's own community.
@visibleForTesting
Future<bool> sendNotificationReply(Ref ref, BuzzNotificationReply reply) async {
  // The background grace period would otherwise disconnect mid-reply.
  final release = ref
      .read(relaySessionProvider.notifier)
      .holdForBackgroundWork(_sendDeadline + _markReadDeadline);
  try {
    // Timing out doesn't stop the send, so it checks this before publishing:
    // a reply kept as a draft must never also go out.
    var expired = false;
    try {
      await _send(ref, reply, expired: () => expired).timeout(
        _sendDeadline,
        onTimeout: () {
          expired = true;
          throw TimeoutException('Reply not sent in time', _sendDeadline);
        },
      );
    } catch (error) {
      debugPrint('[NotificationReply] not sent: $error');
      await _keepDraft(ref, reply);
      return false;
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
    return true;
  } finally {
    release();
  }
}

/// Publishes [reply] to its own community's relay, or throws, including when
/// [expired] says the reply has already been given up on.
Future<void> _send(
  Ref ref,
  BuzzNotificationReply reply, {
  required bool Function() expired,
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
      .connectForBackgroundWork(timeout: _prepareDeadline);
  if (!connected) throw StateError('Relay not reachable');
  if (ref.read(activeCommunityProvider).value?.id != link.communityId) {
    throw StateError('Selected community changed');
  }
  if (expired()) throw StateError('Reply deadline passed');
  // A Solo chat takes no thread replies, so the reply goes at its top level.
  final threaded = !channel.isSolo;
  await send(
    channelId: link.channelId,
    content: reply.text,
    parentEventId: threaded ? link.messageId : null,
    rootEventId: threaded ? reply.replyRootId ?? link.messageId : null,
    mentionPubkeys: [reply.senderPubkey],
    channel: channel,
  );
}

/// Keeps the reply's text in the composer its notification opens, after any
/// draft already there, in the reply's own community.
Future<void> _keepDraft(Ref ref, BuzzNotificationReply reply) async {
  final link = reply.link;
  try {
    final communities = await ref.read(communityListProvider.future);
    final community = communities
        .where((community) => community.id == link.communityId)
        .firstOrNull;
    if (community == null) {
      throw StateError('Community ${link.communityId} not found');
    }
    ref
        .read(composeDraftsProvider.notifier)
        .append(
          config: RelayConfig(
            baseUrl: community.relayUrl,
            nsec: community.nsec,
          ),
          key: composeDraftKey(link.channelId, threadHeadId: link.threadRootId),
          channelId: link.channelId,
          threadHeadId: link.threadRootId,
          text: reply.text,
        );
  } catch (error) {
    debugPrint('[NotificationReply] draft not kept: $error');
  }
}
