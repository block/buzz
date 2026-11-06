import 'dart:math' as math;

import '../../../shared/deeplink/deep_link.dart';

/// The short label a Buzz channel, message, repo, pull request or issue link
/// shows in place of its URL, or null for any other link. [channelName] gives
/// a channel's display name, or null when it is unknown.
String? buzzLinkLabel(
  BuzzDeepLink? link,
  String? Function(String channelId) channelName,
) => switch (link) {
  ChannelDeepLink(:final channelId) => buzzChannelLabel(channelId, channelName),
  MessageDeepLink(:final channelId, :final messageId) =>
    '${buzzChannelLabel(channelId, channelName)} · ${shortBuzzId(messageId)}',
  EntityDeepLink(type: 'repo', :final repository) => repository,
  EntityDeepLink(:final repository, :final eventId) =>
    '$repository · ${shortBuzzId(eventId ?? '')}',
  _ => null,
};

/// [channelId]'s display name, or its first 8 characters when unknown.
String buzzChannelLabel(
  String channelId,
  String? Function(String channelId) channelName,
) => channelName(channelId) ?? shortBuzzId(channelId);

/// The first 8 characters of an id, as Buzz links show it.
String shortBuzzId(String id) => id.substring(0, math.min(8, id.length));
