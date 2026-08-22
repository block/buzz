import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/relay/relay.dart';
import '../channels/channel_management_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/user_profile.dart';
import 'channel.dart';
import 'channel_messages_provider.dart';
import 'message_mention_pubkeys.dart';
import 'local_message_send_animation_provider.dart';

/// Sends messages by signing an event with the user's nsec and publishing it
/// over the relay's NIP-42-authenticated WebSocket session.
class SendMessage {
  final SignedEventRelay _signedEventRelay;
  final Future<List<ChannelMember>> Function(String channelId) _fetchMembers;
  final Map<String, UserProfile> Function() _readUserCache;
  final void Function(String channelId, NostrEvent event) _addLocalMessage;
  final void Function(String channelId, String eventId)
  _markLocalMessageForAnimation;
  final void Function(String channelId, String eventId) _completeLocalMessage;
  final void Function(String channelId, String eventId) _removeLocalMessage;
  final bool Function()? _isDeliveryValid;

  SendMessage({
    required SignedEventRelay signedEventRelay,
    required Future<List<ChannelMember>> Function(String channelId)
    fetchMembers,
    required Map<String, UserProfile> Function() readUserCache,
    required void Function(String channelId, NostrEvent event) addLocalMessage,
    void Function(String channelId, String eventId)?
    markLocalMessageForAnimation,
    required void Function(String channelId, String eventId)
    completeLocalMessage,
    required void Function(String channelId, String eventId) removeLocalMessage,
    bool Function()? isDeliveryValid,
  }) : _signedEventRelay = signedEventRelay,
       _fetchMembers = fetchMembers,
       _readUserCache = readUserCache,
       _addLocalMessage = addLocalMessage,
       _markLocalMessageForAnimation =
           markLocalMessageForAnimation ?? ((_, _) {}),
       _completeLocalMessage = completeLocalMessage,
       _removeLocalMessage = removeLocalMessage,
       _isDeliveryValid = isDeliveryValid;

  /// Send a text message to a channel.
  ///
  /// For thread replies, pass [parentEventId] and optionally [rootEventId].
  /// If [rootEventId] is null it defaults to [parentEventId] (direct reply to
  /// thread head). Tags are built to match the desktop's `buildReplyTags`
  /// convention with `root` / `reply` markers. Pass [mediaTags] to append
  /// relay-validated `imeta` tags and NIP-30 `emoji` tags.
  Future<void> call({
    required String channelId,
    required String content,
    String? parentEventId,
    String? rootEventId,
    List<String>? mentionPubkeys,
    Channel? channel,
    List<List<String>> mediaTags = const [],
  }) async {
    _ensureDeliveryValid();
    // Use explicitly passed pubkeys, or resolve @mentions against
    // channel members to avoid matching the wrong user.
    final explicitMentions =
        mentionPubkeys ?? await _resolveMentions(content, channelId);
    final authorPubkey = _signedEventRelay.pubkey;
    final dmRecipientPubkeys = channel?.isDm == true
        ? await _fetchDmRecipientPubkeys(channelId, channel!, authorPubkey)
        : null;
    final resolvedMentions = dmRecipientPubkeys != null
        ? messageMentionPubkeys(
            channel: channel!,
            senderPubkey: authorPubkey,
            explicitMentions: explicitMentions,
            dmRecipientPubkeys: dmRecipientPubkeys,
          )
        : explicitMentions;

    // Normalize mentions: lowercase, deduplicate, exclude self (matching
    // the desktop's normalizeMentionPubkeys).
    final selfLower = authorPubkey?.toLowerCase();
    final seenMentions = <String>{?selfLower};
    final normalizedMentions = <String>[
      for (final pk in resolvedMentions)
        if (seenMentions.add(pk.toLowerCase())) pk,
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
          _markLocalMessageForAnimation(channelId, event.id);
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

  /// Resolve every identity that is actually a current member of this DM.
  ///
  /// Membership is authoritative for delivery. The channel metadata's `p`
  /// tags can lag membership changes, so they are only used when the membership
  /// snapshot is unavailable.
  Future<Set<String>> _fetchDmRecipientPubkeys(
    String channelId,
    Channel channel,
    String? authorPubkey,
  ) async {
    List<ChannelMember>? members;
    try {
      members = await _fetchMembers(channelId);
    } catch (_) {
      // Fall back to metadata below so an unavailable membership query does
      // not block ordinary DM sends.
    }

    final author = authorPubkey?.toLowerCase();
    final participants = members != null && members.isNotEmpty
        ? members.map((member) => member.pubkey)
        : channel.participantPubkeys;
    return {
      for (final participant in participants)
        if (participant.trim().isNotEmpty &&
            participant.toLowerCase() != author)
          participant.toLowerCase(),
    };
  }

  void _ensureDeliveryValid() {
    if (_isDeliveryValid?.call() == false) {
      throw StateError(
        'Message delivery cancelled because the active community changed',
      );
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
    markLocalMessageForAnimation: (channelId, eventId) => ref
        .read(localMessageSendAnimationProvider(channelId).notifier)
        .mark(eventId),
    completeLocalMessage: (channelId, eventId) => ref
        .read(channelMessagesProvider(channelId).notifier)
        .completeLocalMessage(eventId),
    removeLocalMessage: (channelId, eventId) => ref
        .read(channelMessagesProvider(channelId).notifier)
        .removeLocalMessage(eventId),
    isDeliveryValid: () {
      final currentConfig = ref.read(relayConfigProvider);
      return currentConfig.baseUrl == config.baseUrl &&
          currentConfig.nsec == config.nsec;
    },
  );
});
