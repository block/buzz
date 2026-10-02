part of 'channel_management_provider.dart';

const _huddleReactionNameMax = 48;

String _clampHuddleReactionName(String name) {
  final trimmed = name.trim();
  if (trimmed.length <= _huddleReactionNameMax) return trimmed;
  return '${trimmed.substring(0, _huddleReactionNameMax - 1).trimRight()}…';
}

/// Canonical Huddle lifecycle operations built on the shared channel actions.
extension HuddleChannelActions on ChannelActions {
  /// Creates the private, one-hour stream used only by Huddle media.
  Future<String> createHuddleBackingChannel() async {
    final channelId = _newUuidV4();
    await _signedEventRelay.submit(
      kind: 9007,
      content: '',
      tags: buildCreateChannelTags(
        channelId: channelId,
        name: 'huddle-${channelId.substring(0, 8)}',
        channelType: 'stream',
        visibility: 'private',
        ttlSeconds: 3600,
      ),
    );
    // The accepted kind:9007 write has already committed creator membership
    // on the relay. Avoid refreshing the global channel list here: the backing
    // channel is intentionally hidden, and racing its membership fan-out can
    // temporarily make the parent channel appear read-only.
    return channelId;
  }

  /// Publishes the canonical creator-signed Huddle start event in its parent.
  Future<NostrEvent> announceHuddleStarted({
    required String parentChannelId,
    required String ephemeralChannelId,
  }) async {
    NostrEvent? signed;
    await _signedEventRelay.submit(
      kind: EventKind.huddleStarted,
      content: jsonEncode({'ephemeral_channel_id': ephemeralChannelId}),
      tags: [
        ['h', parentChannelId],
      ],
      onSigned: (event) => signed = event,
    );
    return signed ??
        (throw StateError('Huddle start event was not signed locally'));
  }

  /// Publishes the canonical creator-signed Huddle end event in its parent.
  Future<void> announceHuddleEnded({
    required String parentChannelId,
    required String ephemeralChannelId,
  }) async {
    await _signedEventRelay.submit(
      kind: EventKind.huddleEnded,
      content: jsonEncode({'ephemeral_channel_id': ephemeralChannelId}),
      tags: [
        ['h', parentChannelId],
      ],
    );
  }

  /// Sends an ephemeral emoji burst to the active Huddle backing channel.
  Future<void> sendHuddleReaction({
    required String channelId,
    required String emoji,
    required String senderName,
  }) async {
    final normalizedEmoji = emoji.trim();
    if (normalizedEmoji.isEmpty) return;
    final shortcode = normalizeShortcode(normalizedEmoji);
    final emojiUrl = reactionEmojiUrl(
      normalizedEmoji,
      _ref.read(customEmojiListProvider),
    );
    await _signedEventRelay.submit(
      kind: EventKind.huddleReaction,
      content: normalizedEmoji,
      tags: [
        ['h', channelId],
        ['reaction', normalizedEmoji],
        ['sender_name', _clampHuddleReactionName(senderName)],
        if (shortcode != null && emojiUrl != null)
          ['emoji', shortcode, emojiUrl],
      ],
    );
  }
}
