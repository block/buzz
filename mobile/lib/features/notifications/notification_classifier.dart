import 'dart:convert';

import '../../shared/deeplink/deep_link.dart';
import '../../shared/relay/nostr_models.dart';
import '../../shared/utils/string_utils.dart';
import '../channels/channel.dart';
import '../channels/unread_badge/is_high_priority_event.dart';
import '../channels/unread_badge/should_notify_for_event.dart';
import 'notification_event.dart';

const notificationBodyMaxCharacters = 140;

NotificationEvent? classifyNotificationEvent({
  required NostrEvent event,
  required Channel channel,
  required String myPubkey,
  String? senderName,
  Set<String> participatedRootIds = const {},
  Set<String> followedRootIds = const {},
  Set<String> authoredRootIds = const {},
  Set<String> mutedRootIds = const {},
  Set<String> mutedChannelIds = const {},
}) {
  if (event.id.trim().isEmpty ||
      channel.id.trim().isEmpty ||
      event.channelId != channel.id ||
      !channel.isMember ||
      channel.isArchived) {
    return null;
  }

  if (!shouldNotifyForEvent(
    event,
    myPubkey,
    participatedRootIds: participatedRootIds,
    followedRootIds: followedRootIds,
    authoredRootIds: authoredRootIds,
    mutedRootIds: mutedRootIds,
    mutedChannelIds: mutedChannelIds,
    channelId: channel.id,
  )) {
    return null;
  }

  final isMention = _mentionsPubkey(event.tags, myPubkey);
  final isPriority = channel.isDm || isHighPriorityEvent(event.tags, myPubkey);
  final normalizedSenderName = senderName?.trim();
  final senderLabel = normalizedSenderName?.isNotEmpty == true
      ? normalizedSenderName!
      : shortPubkey(event.pubkey);
  final title = channel.isDm
      ? (normalizedSenderName?.isNotEmpty == true
            ? normalizedSenderName!
            : 'Direct message')
      : isMention
      ? '$senderLabel mentioned you in #${channel.name}'
      : 'New message in #${channel.name}';

  return NotificationEvent(
    eventId: event.id,
    id: notificationIdForEvent(event.id),
    category: isPriority
        ? NotificationCategory.priority
        : NotificationCategory.activity,
    title: title,
    body: trimNotificationBody(event.content),
    route: buildMessageLink(
      channelId: channel.id,
      messageId: event.id,
      threadRootId: event.threadReference.rootId,
    ),
  );
}

String trimNotificationBody(String content) {
  final trimmed = content.trim().replaceAll(RegExp(r'\s+', unicode: true), ' ');
  final runes = trimmed.runes;
  if (runes.length <= notificationBodyMaxCharacters) return trimmed;
  return String.fromCharCodes(
    runes.take(notificationBodyMaxCharacters),
  ).trimRight();
}

int notificationIdForEvent(String eventId) {
  var hash = 0x811c9dc5;
  for (final byte in utf8.encode(eventId)) {
    hash ^= byte;
    hash = (hash * 0x01000193) & 0xffffffff;
  }
  final id = hash & 0x7fffffff;
  return id == 0 ? 1 : id;
}

bool _mentionsPubkey(List<List<String>> tags, String pubkey) {
  final normalizedPubkey = pubkey.toLowerCase();
  return tags.any(
    (tag) =>
        tag.length >= 2 &&
        tag[0] == 'p' &&
        tag[1].toLowerCase() == normalizedPubkey,
  );
}
