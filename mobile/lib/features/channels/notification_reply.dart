import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/auth/auth.dart';
import '../../shared/push/push_bridge.dart';
import '../../shared/read_state/read_state_format.dart';
import '../../shared/read_state/read_state_provider.dart';
import '../../shared/relay/relay.dart';
import '../activity/compose_drafts_provider.dart';
import 'channel.dart';
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

/// How long a reply may take to get ready to publish. iOS gives the app
/// about 30 s and the native side gives up at 25 s, so this leaves room for
/// the publish's own acknowledgement timeout.
const _prepareDeadline = Duration(seconds: 12);

/// Sends [reply] in a subthread under the notified message, mentioning its
/// sender, then marks the conversation read up to that message. Returns
/// whether it was sent; an unsent reply's text is kept as a draft where its
/// notification opens.
@visibleForTesting
Future<bool> sendNotificationReply(Ref ref, BuzzNotificationReply reply) async {
  final link = reply.link;
  try {
    final channel = await _replyChannel(ref, reply).timeout(_prepareDeadline);
    if (channel == null) {
      _keepDraft(ref, reply);
      return false;
    }
    // A Solo chat takes no thread replies, so the reply goes at its top level.
    final threaded = !channel.isSolo;
    await ref
        .read(sendMessageProvider)
        .call(
          channelId: link.channelId,
          content: reply.text,
          parentEventId: threaded ? link.messageId : null,
          rootEventId: threaded ? reply.replyRootId ?? link.messageId : null,
          mentionPubkeys: [reply.senderPubkey],
          channel: channel,
        );
  } catch (error) {
    debugPrint('[NotificationReply] not sent: $error');
    _keepDraft(ref, reply);
    return false;
  }
  try {
    final rootId = reply.replyRootId;
    final readState = ref.read(readStateProvider.notifier);
    readState.markContextRead(
      rootId == null ? link.channelId : threadContextKey(rootId),
      reply.messageCreatedAt,
    );
    // The app may be suspended right after the reply finishes.
    await readState.flush();
  } catch (error) {
    debugPrint('[NotificationReply] not marked read: $error');
  }
  return true;
}

/// The channel to reply in, once the relay can take the reply; null when the
/// reply can't be sent there.
Future<Channel?> _replyChannel(Ref ref, BuzzNotificationReply reply) async {
  final link = reply.link;
  final auth = await ref.read(authProvider.future);
  final active = await ref.read(activeCommunityProvider.future);
  // Replying never switches communities, and only the selected one is
  // reachable.
  if (auth.status != AuthStatus.authenticated ||
      active?.id != link.communityId) {
    return null;
  }
  final channels = await ref.read(channelsProvider.future);
  final channel = channels
      .where((channel) => channel.id == link.channelId)
      .firstOrNull;
  if (channel == null || !channel.isMember) return null;
  // The app's relay connection closes in the background.
  final connected = await ref
      .read(relaySessionProvider.notifier)
      .connectForBackgroundWork(timeout: _prepareDeadline);
  return connected ? channel : null;
}

/// Keeps the reply's text in the composer its notification opens, after any
/// draft already there.
void _keepDraft(Ref ref, BuzzNotificationReply reply) {
  final link = reply.link;
  try {
    final drafts = ref.read(composeDraftsProvider.notifier);
    final key = composeDraftKey(
      link.channelId,
      threadHeadId: link.threadRootId,
    );
    final existing = drafts.draftFor(key);
    final existingText = existing?.text.trimRight() ?? '';
    drafts.save(
      key: key,
      channelId: link.channelId,
      threadHeadId: link.threadRootId,
      text: existingText.isEmpty
          ? reply.text
          : '$existingText\n\n${reply.text}',
      mentionKeys: existing?.mentionKeys ?? const {},
    );
  } catch (error) {
    debugPrint('[NotificationReply] draft not kept: $error');
  }
}
