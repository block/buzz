part of 'channel_management_provider.dart';

/// Builds the relay tags for setting the archived state of [channelId].
List<List<String>> buildSetChannelArchivedTags(
  String channelId, {
  required bool archived,
}) => [
  ['h', channelId],
  ['archived', archived.toString()],
];

/// Builds the relay tags for deleting [channelId].
List<List<String>> buildDeleteChannelTags(String channelId) => [
  ['h', channelId],
];

class ChannelActions {
  final Ref _ref;
  final RelaySessionNotifier _session;
  final SignedEventRelay _signedEventRelay;
  final String? _currentPubkey;
  final bool Function()? _isCommunityValid;

  ChannelActions({
    required Ref ref,
    required RelaySessionNotifier session,
    required SignedEventRelay signedEventRelay,
    required String? currentPubkey,
    bool Function()? isCommunityValid,
  }) : _ref = ref,
       _session = session,
       _signedEventRelay = signedEventRelay,
       _currentPubkey = currentPubkey,
       _isCommunityValid = isCommunityValid;

  Future<Channel> createChannel({
    required String name,
    required String channelType,
    required String visibility,
    String? description,
    int? ttlSeconds,
  }) async {
    final channelId = _newUuidV4();
    final tags = buildCreateChannelTags(
      channelId: channelId,
      name: name,
      channelType: channelType,
      visibility: visibility,
      description: description,
      ttlSeconds: ttlSeconds,
    );
    _ensureCommunityValid();
    await _signedEventRelay.submit(kind: 9007, content: '', tags: tags);
    _ensureCommunityValid();
    return _refreshChannelsAndRead(channelId);
  }

  /// Open (or create) a DM channel with the given pubkeys.
  ///
  /// This submits a kind:41010 command event; the relay responds with an OK
  /// message whose content carries `response:{...}` containing the new
  /// `channel_id`.
  Future<Channel> openDm({required List<String> pubkeys}) async {
    _ensureCommunityValid();
    final result = await _signedEventRelay.submit(
      kind: 41010,
      content: '',
      tags: pubkeys.map((pk) => ['p', pk]).toList(),
    );
    _ensureCommunityValid();
    final response = parseCommandResponse(result.content);
    final channelId = response?['channel_id'] as String?;
    if (channelId == null || channelId.isEmpty) {
      throw Exception('Relay did not return a DM channel id');
    }
    return _refreshChannelsAndRead(channelId);
  }

  Future<void> addMembers({
    required String channelId,
    required List<String> pubkeys,
    String role = 'member',
  }) async {
    final normalizedRole = role.trim();
    if (normalizedRole.isEmpty) {
      throw ArgumentError.value(role, 'role', 'must not be empty');
    }
    final normalizedPubkeys = {
      for (final pubkey in pubkeys)
        if (pubkey.trim().isNotEmpty) pubkey.trim().toLowerCase(),
    };
    _ensureCommunityValid();
    // Per-pubkey failures are collected rather than thrown on the spot: one
    // relay rejection must not skip the remaining adds or the invalidation
    // below, which would leave the members list stale for the adds that landed.
    final failures = <String, String>{};
    for (final pubkey in normalizedPubkeys) {
      // Outside the catch: a community switch mid-loop must abort the whole
      // add, not be recorded as this pubkey's rejection.
      _ensureCommunityValid();
      try {
        await _signedEventRelay.submit(
          kind: 9000,
          content: '',
          tags: [
            ['h', channelId],
            ['p', pubkey],
            ['role', normalizedRole],
          ],
        );
      } catch (error) {
        failures[pubkey] = _relayErrorMessage(error);
      }
    }
    _ensureCommunityValid();
    _ref.invalidate(channelMembersProvider(channelId));
    _ref.invalidate(channelBotPubkeysProvider(channelId));
    if (failures.isNotEmpty) {
      throw AddMembersException(failures);
    }
  }

  void _ensureCommunityValid() {
    if (_isCommunityValid?.call() == false) {
      throw StateError(
        'Channel action cancelled because the active community changed',
      );
    }
  }

  Future<void> joinChannel(String channelId) async {
    _ensureCommunityValid();
    await _signedEventRelay.submit(
      kind: 9021,
      content: '',
      tags: [
        ['h', channelId],
      ],
    );
    _ensureCommunityValid();
    await _refreshChannelState(channelId);
  }

  Future<void> leaveChannel(String channelId) async {
    _ensureCommunityValid();
    await _signedEventRelay.submit(
      kind: 9022,
      content: '',
      tags: [
        ['h', channelId],
      ],
    );
    _ensureCommunityValid();
    await _refreshChannelState(channelId);
  }

  /// Updates the user-editable channel metadata and refreshes cached details.
  Future<void> updateChannel({
    required String channelId,
    String? name,
    String? description,
  }) async {
    _ensureCommunityValid();
    await _signedEventRelay.submit(
      kind: 9002,
      content: '',
      tags: buildUpdateChannelTags(
        channelId: channelId,
        name: name,
        description: description,
      ),
    );
    _ensureCommunityValid();
    await _refreshChannelState(channelId);
  }

  /// Archives the channel and refreshes its cached state.
  Future<void> archiveChannel(String channelId) =>
      _setChannelArchived(channelId, archived: true);

  /// Unarchives the channel and refreshes its cached state.
  Future<void> unarchiveChannel(String channelId) =>
      _setChannelArchived(channelId, archived: false);

  Future<void> _setChannelArchived(
    String channelId, {
    required bool archived,
  }) async {
    _ensureCommunityValid();
    await _signedEventRelay.submit(
      kind: 9002,
      content: '',
      tags: buildSetChannelArchivedTags(channelId, archived: archived),
    );
    _ensureCommunityValid();
    await _refreshChannelState(channelId);
  }

  /// Deletes the channel and refreshes its cached state.
  Future<void> deleteChannel(String channelId) async {
    _ensureCommunityValid();
    await _signedEventRelay.submit(
      kind: 9008,
      content: '',
      tags: buildDeleteChannelTags(channelId),
    );
    _ensureCommunityValid();
    await _refreshChannelState(channelId);
  }

  Future<void> setCanvas({
    required String channelId,
    required String content,
  }) async {
    _ensureCommunityValid();
    await _signedEventRelay.submit(
      kind: 40100,
      content: content,
      tags: [
        ['h', channelId],
      ],
    );
    _ensureCommunityValid();
    _ref.invalidate(channelCanvasProvider(channelId));
  }

  /// User search via NIP-50 over kind:0 profile events.
  Future<List<DirectoryUser>> searchUsers(String query, {int limit = 8}) async {
    final trimmed = query.trim();
    if (trimmed.isEmpty) return const [];

    final events = await _session.queryRelay([
      NostrFilters.searchUsers(trimmed, limit: limit),
    ]);
    return directoryUsersFromProfileEvents(events)
        .where(
          (user) =>
              _currentPubkey == null ||
              user.pubkey.toLowerCase() != _currentPubkey,
        )
        .toList();
  }

  Future<Channel> _refreshChannelsAndRead(String channelId) async {
    _ensureCommunityValid();
    await _ref.read(channelsProvider.notifier).refresh();
    _ensureCommunityValid();
    final channels = await _ref.read(channelsProvider.future);
    _ensureCommunityValid();
    return channels.firstWhere(
      (channel) => channel.id == channelId,
      orElse: () =>
          throw Exception('Channel was created but is not visible yet'),
    );
  }

  Future<void> _refreshChannelState(String channelId) async {
    _ensureCommunityValid();
    await _ref.read(channelsProvider.notifier).refresh();
    _ensureCommunityValid();
    _ref.invalidate(channelDetailsProvider(channelId));
    _ref.invalidate(channelMembersProvider(channelId));
    _ref.invalidate(channelBotPubkeysProvider(channelId));
    _ref.invalidate(channelCanvasProvider(channelId));
  }

  String _newUuidV4() {
    final bytes = List<int>.generate(16, (_) => _random.nextInt(256));
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;

    final hex = bytes
        .map((byte) => byte.toRadixString(16).padLeft(2, '0'))
        .join();
    return '${hex.substring(0, 8)}-'
        '${hex.substring(8, 12)}-'
        '${hex.substring(12, 16)}-'
        '${hex.substring(16, 20)}-'
        '${hex.substring(20, 32)}';
  }

  Future<void> changeMemberRole({
    required String channelId,
    required String pubkey,
    required String role,
  }) async {
    _ensureCommunityValid();
    await _signedEventRelay.submit(
      kind: 9000,
      content: '',
      tags: [
        ['h', channelId],
        ['p', pubkey.toLowerCase()],
        ['role', role],
      ],
    );
    _ensureCommunityValid();
    _ref.invalidate(channelMembersProvider(channelId));
    _ref.invalidate(channelBotPubkeysProvider(channelId));
  }

  Future<void> removeMember({
    required String channelId,
    required String pubkey,
  }) async {
    _ensureCommunityValid();
    await _signedEventRelay.submit(
      kind: 9001,
      content: '',
      tags: [
        ['h', channelId],
        ['p', pubkey.toLowerCase()],
      ],
    );
    _ensureCommunityValid();
    _ref.invalidate(channelMembersProvider(channelId));
    _ref.invalidate(channelBotPubkeysProvider(channelId));
  }

  Future<void> addReaction(String eventId, String emoji) async {
    final shortcode = normalizeShortcode(emoji);
    final emojiUrl = reactionEmojiUrl(
      emoji,
      _ref.read(customEmojiListProvider),
    );
    await _signedEventRelay.submit(
      kind: EventKind.reaction,
      content: emoji,
      tags: [
        ['e', eventId],
        if (shortcode != null && emojiUrl != null)
          ['emoji', shortcode, emojiUrl],
      ],
    );
  }

  Future<void> removeReaction(String reactionEventId, String emoji) async {
    await _signedEventRelay.submit(
      kind: EventKind.deletion,
      content: '',
      tags: [
        ['e', reactionEventId],
      ],
    );
  }

  Future<void> editMessage({
    required String channelId,
    required String eventId,
    required String content,
    List<List<String>> mediaTags = const [],
  }) async {
    _ensureCommunityValid();
    await _signedEventRelay.submit(
      kind: EventKind.streamMessageEdit,
      content: content,
      tags: [
        ['h', channelId],
        ['e', eventId],
        ...mediaTags,
      ],
    );
    _ensureCommunityValid();
  }

  Future<void> deleteMessage({
    required String channelId,
    required String eventId,
  }) async {
    _ensureCommunityValid();
    await _signedEventRelay.submit(
      kind: EventKind.deletion,
      content: '',
      tags: buildDeleteMessageTags(channelId: channelId, eventId: eventId),
    );
    _ensureCommunityValid();
  }

  static final Random _random = Random.secure();
}

final channelActionsProvider = Provider<ChannelActions>((ref) {
  final relayConfig = ref.watch(relayConfigProvider);
  final currentPubkey = ref.watch(currentPubkeyProvider);
  final session = ref.read(relaySessionProvider.notifier);
  return ChannelActions(
    ref: ref,
    session: session,
    signedEventRelay: SignedEventRelay(
      session: session,
      nsec: relayConfig.nsec,
    ),
    currentPubkey: currentPubkey,
    isCommunityValid: () {
      final currentConfig = ref.read(relayConfigProvider);
      return currentConfig.baseUrl == relayConfig.baseUrl &&
          currentConfig.nsec == relayConfig.nsec;
    },
  );
});
