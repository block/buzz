import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/mentions/mention_tags.dart';
import '../../shared/relay/relay.dart';
import '../channels/channel_management_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import '../../shared/profile/user_profile.dart';
import 'channel.dart';
import 'me/me_messages_provider.dart';
import 'channel_messages_provider.dart';
import 'message_mention_pubkeys.dart';
import 'local_message_send_animation_provider.dart';
import 'local_send_status_provider.dart';
import 'pending_local_messages_provider.dart';

/// Relays reject events dated more than 15 minutes from their clock.
const _republishWindowSeconds = 10 * 60;

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
  final void Function(String channelId, String eventId)? _failLocalMessage;
  final NostrEvent? Function(String channelId, String eventId)
  _readPendingLocalMessage;
  final void Function(String channelId, String eventId) _trackLocalMessage;
  final void Function(String channelId, String eventId) _retryLocalMessage;
  final void Function(String channelId, String eventId) _forgetLocalMessage;
  final Future<void> Function(NostrEvent event)? _publishSigned;
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
    NostrEvent? Function(String channelId, String eventId)?
    readPendingLocalMessage,
    void Function(String channelId, String eventId)? trackLocalMessage,
    void Function(String channelId, String eventId)? failLocalMessage,
    void Function(String channelId, String eventId)? retryLocalMessage,
    void Function(String channelId, String eventId)? forgetLocalMessage,
    Future<void> Function(NostrEvent event)? publishSigned,
    bool Function()? isDeliveryValid,
  }) : _signedEventRelay = signedEventRelay,
       _fetchMembers = fetchMembers,
       _readUserCache = readUserCache,
       _addLocalMessage = addLocalMessage,
       _markLocalMessageForAnimation =
           markLocalMessageForAnimation ?? ((_, _) {}),
       _completeLocalMessage = completeLocalMessage,
       _removeLocalMessage = removeLocalMessage,
       _readPendingLocalMessage = readPendingLocalMessage ?? ((_, _) => null),
       _trackLocalMessage = trackLocalMessage ?? ((_, _) {}),
       _failLocalMessage = failLocalMessage,
       _retryLocalMessage = retryLocalMessage ?? ((_, _) {}),
       _forgetLocalMessage = forgetLocalMessage ?? ((_, _) {}),
       _publishSigned = publishSigned,
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

    // Two-field `mention` tags are references; they go through the same
    // writer as recipients so a bad key rejects the message.
    bool isReference(List<String> tag) =>
        tag.length == 2 && tag[0] == 'mention';
    final references = [
      for (final tag in mediaTags)
        if (isReference(tag)) tag[1],
    ];
    final mentionTags = writeMentionTags(
      recipients: normalizedMentions,
      references: references,
      members: normalizedMentions.isEmpty
          ? const {}
          : await _recipientMembers(channelId, dmRecipientPubkeys, selfLower),
    );

    final tags = <List<String>>[
      ['h', channelId],
      if (parentEventId != null) ..._buildReplyTags(parentEventId, rootEventId),
      ...mentionTags,
      for (final tag in mediaTags)
        if (!isReference(tag)) tag,
    ];

    await _sendSigned(channelId, content, tags);
  }

  Future<void> _sendSigned(
    String channelId,
    String content,
    List<List<String>> tags,
  ) async {
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
          _trackLocalMessage(channelId, event.id);
          _addLocalMessage(channelId, event);
        },
      );
      final event = localMessage;
      if (event != null) _completeLocalMessage(channelId, event.id);
    } catch (_) {
      final event = localMessage;
      if (event == null) rethrow;
      final fail = _failLocalMessage;
      if (fail == null) {
        _removeLocalMessage(channelId, event.id);
        rethrow;
      }
      // The signed bubble stays in the conversation as the retry record:
      // the user can resend these exact bytes or delete it.
      fail(channelId, event.id);
    }
  }

  /// Resends a failed message.
  ///
  /// A recent failure republishes the exact signed event, so a publish the
  /// relay already accepted (an acknowledgement lost to a timeout)
  /// deduplicates instead of repeating. The relay rejects events dated far
  /// from its clock, so an older failure is signed again as a new message at
  /// the end of the conversation, replacing the failed bubble.
  Future<void> retry({
    required String channelId,
    required String eventId,
  }) async {
    final event = _readPendingLocalMessage(channelId, eventId);
    // Delivered meanwhile (a relay echo arrived), or already deleted.
    if (event == null) return;
    final publish = _publishSigned;
    if (publish == null) {
      throw StateError('This sender cannot republish signed messages.');
    }
    final ageSeconds =
        DateTime.now().millisecondsSinceEpoch ~/ 1000 - event.createdAt;
    if (ageSeconds > _republishWindowSeconds) {
      discard(channelId: channelId, eventId: eventId);
      await _sendSigned(channelId, event.content, event.tags);
      return;
    }
    _retryLocalMessage(channelId, eventId);
    try {
      _ensureDeliveryValid();
      await publish(event);
      _completeLocalMessage(channelId, eventId);
    } catch (_) {
      (_failLocalMessage ?? _removeLocalMessage)(channelId, eventId);
    }
  }

  /// Deletes a failed message that was never delivered.
  void discard({required String channelId, required String eventId}) {
    _removeLocalMessage(channelId, eventId);
    _forgetLocalMessage(channelId, eventId);
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
    return {
      for (final participant in dmParticipantPubkeys(channel, members))
        if (participant.trim().isNotEmpty &&
            participant.toLowerCase() != author)
          participant.toLowerCase(),
    };
  }

  /// Keys that can be recipients: DM participants, or the channel's current
  /// members. The sender always counts as a member.
  Future<Set<String>> _recipientMembers(
    String channelId,
    Set<String>? dmRecipientPubkeys,
    String? self,
  ) async {
    final members =
        dmRecipientPubkeys ??
        {
          for (final member in await _fetchMembers(channelId))
            member.pubkey.toLowerCase(),
        };
    return {...members, ?self};
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

final sendMessageProvider = Provider<SendMessage>((ref) => _sender(ref));

/// Shares signing and delivery with channels, with Me's flat optimistic reader.
final meSendMessageProvider = Provider<SendMessage>(
  (ref) => _sender(ref, personal: true),
);

SendMessage _sender(Ref ref, {bool personal = false}) {
  final messages = personal ? meMessagesProvider : channelMessagesProvider;
  final config = ref.watch(relayConfigProvider);
  return SendMessage(
    signedEventRelay: SignedEventRelay(
      session: ref.read(relaySessionProvider.notifier),
      nsec: config.nsec,
    ),
    // A loaded member list is enough to build tags. Waiting on a refresh
    // here would hold the outgoing bubble back until the network answers.
    fetchMembers: (channelId) {
      final members = ref.read(channelMembersProvider(channelId));
      if (members.hasValue) return Future.value(members.requireValue);
      return ref.read(channelMembersProvider(channelId).future);
    },
    readUserCache: () => ref.read(userCacheProvider),
    addLocalMessage: (channelId, event) =>
        ref.read(messages(channelId).notifier).addLocalMessage(event),
    markLocalMessageForAnimation: (channelId, eventId) => ref
        .read(localMessageSendAnimationProvider(channelId).notifier)
        .mark(eventId),
    completeLocalMessage: (channelId, eventId) =>
        ref.read(messages(channelId).notifier).completeLocalMessage(eventId),
    removeLocalMessage: (channelId, eventId) =>
        ref.read(messages(channelId).notifier).removeLocalMessage(eventId),
    publishSigned: (event) =>
        ref.read(relaySessionProvider.notifier).publish(event),
    readPendingLocalMessage: (channelId, eventId) =>
        ref.read(pendingLocalMessagesProvider(channelId))[eventId],
    trackLocalMessage: (channelId, eventId) => ref
        .read(localSendStatusProvider(channelId).notifier)
        .track(eventId, personal: personal),
    failLocalMessage: (channelId, eventId) =>
        ref.read(localSendStatusProvider(channelId).notifier).fail(eventId),
    retryLocalMessage: (channelId, eventId) =>
        ref.read(localSendStatusProvider(channelId).notifier).retrying(eventId),
    forgetLocalMessage: (channelId, eventId) =>
        ref.read(localSendStatusProvider(channelId).notifier).forget(eventId),
    isDeliveryValid: () {
      final currentConfig = ref.read(relayConfigProvider);
      return currentConfig.baseUrl == config.baseUrl &&
          currentConfig.nsec == config.nsec;
    },
  );
}
