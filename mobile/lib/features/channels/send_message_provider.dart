import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../channels/channel_management_provider.dart';
import '../profile/user_cache_provider.dart';
import '../profile/user_profile.dart';
import 'channel_messages_provider.dart';

/// Sends messages by signing an event with the user's nsec and publishing it
/// over the relay's NIP-42-authenticated WebSocket session.
class SendMessage {
  final SignedEventRelay _signedEventRelay;
  final Future<List<ChannelMember>> Function(String channelId) _fetchMembers;
  final Map<String, UserProfile> Function() _readUserCache;
  final void Function(String channelId, NostrEvent event) _addLocalMessage;
  final void Function(String channelId, String eventId) _completeLocalMessage;
  final void Function(String channelId, String eventId) _removeLocalMessage;
  final bool Function()? _isDeliveryValid;
  final Future<bool> Function(String channelId)? _resolveIsDirectMessage;

  SendMessage({
    required SignedEventRelay signedEventRelay,
    required Future<List<ChannelMember>> Function(String channelId)
    fetchMembers,
    required Map<String, UserProfile> Function() readUserCache,
    required void Function(String channelId, NostrEvent event) addLocalMessage,
    required void Function(String channelId, String eventId)
    completeLocalMessage,
    required void Function(String channelId, String eventId) removeLocalMessage,
    bool Function()? isDeliveryValid,
    Future<bool> Function(String channelId)? resolveIsDirectMessage,
  }) : _signedEventRelay = signedEventRelay,
       _fetchMembers = fetchMembers,
       _readUserCache = readUserCache,
       _addLocalMessage = addLocalMessage,
       _completeLocalMessage = completeLocalMessage,
       _removeLocalMessage = removeLocalMessage,
       _isDeliveryValid = isDeliveryValid,
       _resolveIsDirectMessage = resolveIsDirectMessage;

  /// Send a text message to a channel.
  ///
  /// For thread replies, pass [parentEventId] and optionally [rootEventId].
  /// If [rootEventId] is null it defaults to [parentEventId] (direct reply to
  /// thread head). Tags are built to match the desktop's `buildReplyTags`
  /// convention with `root` / `reply` markers. Pass [mediaTags] to append
  /// relay-validated `imeta` tags and NIP-30 `emoji` tags. Direct-message
  /// callers can pass [isDirectMessage] and cached
  /// [directMessageRecipientPubkeys]; otherwise the provider resolves the
  /// channel and its members. Agent wakeups and human notifications rely on
  /// recipient `p` tags even without an explicit `@mention` in the content.
  Future<void> call({
    required String channelId,
    required String content,
    String? parentEventId,
    String? rootEventId,
    List<String>? mentionPubkeys,
    bool? isDirectMessage,
    List<String> directMessageRecipientPubkeys = const [],
    List<List<String>> mediaTags = const [],
  }) async {
    _ensureDeliveryValid();
    // Use explicitly passed pubkeys, or resolve @mentions against
    // channel members to avoid matching the wrong user.
    final resolvedMentions =
        mentionPubkeys ?? await _resolveMentions(content, channelId);
    final authorPubkey = _signedEventRelay.pubkey;
    final sendsDirectMessage =
        isDirectMessage ?? await _channelIsDirectMessage(channelId);

    final recipients = <String>[
      ...resolvedMentions,
      if (sendsDirectMessage) ...directMessageRecipientPubkeys,
    ];

    // A newly discovered DM can briefly lack cached participant pubkeys. Fall
    // back to the authoritative member list so its first plain message still
    // addresses the other participant. Failure remains non-fatal: publishing
    // the message is preferable to losing it because recipient metadata is
    // temporarily unavailable.
    if (sendsDirectMessage && directMessageRecipientPubkeys.isEmpty) {
      try {
        recipients.addAll(
          (await _fetchMembers(channelId)).map((member) => member.pubkey),
        );
      } catch (_) {
        // Non-fatal — publish with any explicit mentions already resolved.
      }
    }

    // Normalize recipients: lowercase, deduplicate, exclude self (matching
    // the desktop's messageMentionPubkeys/normalizeMentionPubkeys path).
    final selfLower = authorPubkey?.toLowerCase();
    final seenMentions = <String>{?selfLower};
    final normalizedMentions = <String>[
      for (final pk in recipients)
        if (pk.isNotEmpty && seenMentions.add(pk.toLowerCase()))
          pk.toLowerCase(),
    ];

    final tags = <List<String>>[
      ['h', channelId],
      if (parentEventId != null) ..._buildReplyTags(parentEventId, rootEventId),
      for (final pk in normalizedMentions) ['p', pk],
      ...mediaTags,
    ];

    _ensureDeliveryValid();
    NostrEvent? localMessage;
    try {
      await _signedEventRelay.submit(
        kind: EventKind.streamMessage,
        content: content,
        tags: tags,
        onSigned: (event) {
          localMessage = event;
          _addLocalMessage(channelId, event);
        },
      );
      final event = localMessage;
      if (event != null) _completeLocalMessage(channelId, event.id);
    } catch (_) {
      final event = localMessage;
      if (event != null) _removeLocalMessage(channelId, event.id);
      rethrow;
    }
  }

  void _ensureDeliveryValid() {
    if (_isDeliveryValid?.call() == false) {
      throw StateError(
        'Message delivery cancelled because the active community changed',
      );
    }
  }

  Future<bool> _channelIsDirectMessage(String channelId) async {
    final resolve = _resolveIsDirectMessage;
    if (resolve == null) return false;
    try {
      return await resolve(channelId);
    } catch (_) {
      return false;
    }
  }

  /// Resolve @mentions to pubkeys, scoped to channel members.
  ///
  /// Fetches channel members from the relay and matches @names only
  /// against members of that channel. Falls back to the full user cache
  /// if the member fetch fails.
  Future<List<String>> _resolveMentions(
    String content,
    String channelId,
  ) async {
    final mentionPattern = RegExp(r'@(\w+)');
    final matches = mentionPattern.allMatches(content);
    if (matches.isEmpty) return const [];

    // Try to get channel member pubkeys for scoped resolution.
    Set<String>? memberPubkeys;
    try {
      final members = await _fetchMembers(channelId);
      memberPubkeys = {for (final m in members) m.pubkey.toLowerCase()};
    } catch (_) {
      // Non-fatal — fall through to unscoped cache lookup.
    }

    final cache = _readUserCache();
    final pubkeys = <String>{};

    for (final match in matches) {
      final name = match.group(1)?.toLowerCase();
      if (name == null || name.isEmpty) continue;

      for (final profile in cache.values) {
        final displayName = profile.displayName?.toLowerCase();
        if (displayName == null) continue;

        // Match against full display name or first word.
        final firstName = displayName.split(RegExp(r'\s+')).first;
        if (displayName != name && firstName != name) continue;

        // If we have channel members, only match members of this channel.
        if (memberPubkeys != null &&
            !memberPubkeys.contains(profile.pubkey.toLowerCase())) {
          continue;
        }

        pubkeys.add(profile.pubkey);
        break;
      }
    }

    return pubkeys.toList();
  }

  /// Build `e`-tags for a thread reply, matching the desktop convention:
  /// - Direct reply to thread head: `["e", id, "", "reply"]`
  /// - Nested reply: `["e", rootId, "", "root"]` + `["e", parentId, "", "reply"]`
  static List<List<String>> _buildReplyTags(
    String parentEventId,
    String? rootEventId,
  ) {
    final root = rootEventId ?? parentEventId;
    if (parentEventId == root) {
      return [
        ['e', root, '', 'reply'],
      ];
    }
    return [
      ['e', root, '', 'root'],
      ['e', parentEventId, '', 'reply'],
    ];
  }
}

final sendMessageProvider = Provider<SendMessage>((ref) {
  final config = ref.watch(relayConfigProvider);
  return SendMessage(
    signedEventRelay: SignedEventRelay(
      session: ref.read(relaySessionProvider.notifier),
      nsec: config.nsec,
    ),
    fetchMembers: (channelId) =>
        ref.read(channelMembersProvider(channelId).future),
    readUserCache: () => ref.read(userCacheProvider),
    addLocalMessage: (channelId, event) => ref
        .read(channelMessagesProvider(channelId).notifier)
        .addLocalMessage(event),
    completeLocalMessage: (channelId, eventId) => ref
        .read(channelMessagesProvider(channelId).notifier)
        .completeLocalMessage(eventId),
    removeLocalMessage: (channelId, eventId) => ref
        .read(channelMessagesProvider(channelId).notifier)
        .removeLocalMessage(eventId),
    resolveIsDirectMessage: (channelId) async =>
        (await ref.read(
          channelDetailsProvider(channelId).future,
        )).channelType ==
        'dm',
    isDeliveryValid: () {
      final currentConfig = ref.read(relayConfigProvider);
      return currentConfig.baseUrl == config.baseUrl &&
          currentConfig.nsec == config.nsec;
    },
  );
});
