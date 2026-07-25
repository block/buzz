import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/auth/auth.dart';
import '../../shared/relay/relay.dart';
import '../profile/profile_provider.dart';
import '../custom_emoji/custom_emoji.dart';
import '../custom_emoji/custom_emoji_provider.dart';
import 'channel.dart';
import 'channels_provider.dart';

@immutable
class ChannelMember {
  final String pubkey;
  final String role;
  final DateTime joinedAt;
  final String? displayName;

  const ChannelMember({
    required this.pubkey,
    required this.role,
    required this.joinedAt,
    this.displayName,
  });

  bool get isBot => role == 'bot';
  bool get isOwner => role == 'owner';
  bool get isElevated => role == 'owner' || role == 'admin';

  String labelFor(String? currentPubkey) {
    if (currentPubkey != null &&
        currentPubkey.toLowerCase() == pubkey.toLowerCase()) {
      return 'You';
    }
    if (displayName case final name? when name.trim().isNotEmpty) {
      return name.trim();
    }
    return pubkey.length > 8 ? '${pubkey.substring(0, 8)}…' : pubkey;
  }
}

@immutable
class ChannelCanvas {
  final String? content;
  final DateTime? updatedAt;
  final String? authorPubkey;

  const ChannelCanvas({
    required this.content,
    required this.updatedAt,
    required this.authorPubkey,
  });
}

@immutable
class DirectoryUser {
  final String pubkey;
  final String? displayName;
  final String? avatarUrl;
  final String? nip05Handle;

  const DirectoryUser({
    required this.pubkey,
    this.displayName,
    this.avatarUrl,
    this.nip05Handle,
  });

  String get label {
    final display = displayName?.trim();
    if (display != null && display.isNotEmpty) {
      return display;
    }
    final nip05 = nip05Handle?.trim();
    if (nip05 != null && nip05.isNotEmpty) {
      return nip05;
    }
    return pubkey.length > 8 ? '${pubkey.substring(0, 8)}…' : pubkey;
  }

  String get secondaryLabel {
    final nip05 = nip05Handle?.trim();
    if (nip05 != null && nip05.isNotEmpty && nip05 != label) {
      return nip05;
    }
    return pubkey.length > 16 ? '${pubkey.substring(0, 16)}…' : pubkey;
  }

  /// First visible character used when no avatar image is available.
  String get initial => label.isNotEmpty ? label[0].toUpperCase() : '?';
}

/// Whether the mobile DM directory should show local preview identities.
const bool mockDmDirectoryEnabled =
    kDebugMode && bool.fromEnvironment('BUZZ_MOCK_DM_DIRECTORY');

/// Whether the new-DM picker should use local preview identities.
///
/// Debug builds fall back automatically while disconnected. Production builds
/// always use the active relay directory.
final dmDirectoryPreviewEnabledProvider = Provider<bool>((ref) {
  if (mockDmDirectoryEnabled) {
    return true;
  }
  final relayStatus = ref.watch(
    relaySessionProvider.select((session) => session.status),
  );
  return kDebugMode && relayStatus != SessionStatus.connected;
});

String _mockEmojiAvatar(String emoji, String color) {
  return Uri.dataFromString(
    '<svg xmlns="http://www.w3.org/2000/svg" width="128" height="128">'
    '<rect width="128" height="128" fill="$color"/>'
    '<text x="64" y="80" text-anchor="middle" font-size="58">$emoji</text>'
    '</svg>',
    mimeType: 'image/svg+xml',
    encoding: utf8,
  ).toString();
}

/// Local identities used to preview the new-DM picker without a relay.
final dmDirectoryPreviewUsers = List<DirectoryUser>.unmodifiable([
  DirectoryUser(
    pubkey: '1111111111111111111111111111111111111111111111111111111111111111',
    displayName: 'Maya Chen',
    avatarUrl: _mockEmojiAvatar('🎨', '#8AADF4'),
    nip05Handle: 'maya@demo.buzz',
  ),
  DirectoryUser(
    pubkey: '2222222222222222222222222222222222222222222222222222222222222222',
    displayName: 'Jordan Brooks',
    avatarUrl: _mockEmojiAvatar('🌱', '#A6DA95'),
    nip05Handle: 'jordan@demo.buzz',
  ),
  const DirectoryUser(
    pubkey: '3333333333333333333333333333333333333333333333333333333333333333',
    displayName: 'Priya Shah',
    nip05Handle: 'priya@demo.buzz',
  ),
  DirectoryUser(
    pubkey: '4444444444444444444444444444444444444444444444444444444444444444',
    displayName: 'Theo Martin',
    avatarUrl: _mockEmojiAvatar('💻', '#C6A0F6'),
    nip05Handle: 'theo@demo.buzz',
  ),
  const DirectoryUser(
    pubkey: '5555555555555555555555555555555555555555555555555555555555555555',
    displayName: 'Sam Rivera',
    nip05Handle: 'sam@demo.buzz',
  ),
]);

final currentPubkeyProvider = Provider<String?>((ref) {
  // Prefer the explicitly-derived pubkey from nsec — this is the signing
  // identity used for events.
  final myPk = ref.watch(myPubkeyProvider);
  if (myPk != null) return myPk.toLowerCase();

  final profile = ref.watch(profileProvider).whenData((value) => value).value;
  final profilePubkey = profile?.pubkey.trim();
  if (profilePubkey != null && profilePubkey.isNotEmpty) {
    return profilePubkey.toLowerCase();
  }

  final authState = ref.watch(authProvider).whenData((value) => value).value;
  final credentialPubkey = authState?.community?.pubkey?.trim();
  if (credentialPubkey != null && credentialPubkey.isNotEmpty) {
    return credentialPubkey.toLowerCase();
  }

  return null;
});

/// Extracts the unique member pubkeys advertised by relay membership events.
///
/// Buzz relays use `member` tags, while older NIP-29-compatible relays may
/// still expose the same directory through `p` tags.
@visibleForTesting
List<String> relayMemberPubkeysFromEvents(List<NostrEvent> events) {
  final pubkeys = <String>{};
  for (final event in events) {
    for (final tag in event.tags) {
      if (tag.length < 2 || (tag[0] != 'member' && tag[0] != 'p')) {
        continue;
      }
      final pubkey = tag[1].trim().toLowerCase();
      if (pubkey.isNotEmpty) {
        pubkeys.add(pubkey);
      }
    }
  }
  return pubkeys.toList();
}

/// Converts kind:0 events into a deduplicated, alphabetized people directory.
@visibleForTesting
List<DirectoryUser> directoryUsersFromProfileEvents(List<NostrEvent> events) {
  final latestByPubkey = <String, NostrEvent>{};
  for (final event in events) {
    if (event.kind != 0) {
      continue;
    }
    final pubkey = event.pubkey.toLowerCase();
    final current = latestByPubkey[pubkey];
    if (current == null || event.createdAt > current.createdAt) {
      latestByPubkey[pubkey] = event;
    }
  }

  return [
    for (final event in latestByPubkey.values)
      if (ProfileData.fromEvent(event) case final profile)
        DirectoryUser(
          pubkey: profile.pubkey.toLowerCase(),
          displayName: profile.displayName,
          avatarUrl: profile.avatarUrl,
          nip05Handle: profile.nip05,
        ),
  ]..sort((a, b) {
    final labelComparison = a.label.toLowerCase().compareTo(
      b.label.toLowerCase(),
    );
    return labelComparison != 0
        ? labelComparison
        : a.pubkey.compareTo(b.pubkey);
  });
}

/// People and agents discoverable on the active relay.
///
/// This mirrors desktop's empty-query people directory by listing kind:0
/// profiles through the HTTP bridge. The relay membership snapshot remains a
/// fallback for older relays that do not support directory listing.
///
/// autoDispose so the cached listing is dropped when the New message sheet
/// closes, and the [relayConfigProvider] watch invalidates it at the
/// community boundary. [relaySessionProvider.notifier] is a stable Notifier
/// instance and [currentPubkeyProvider] keeps its value when two communities
/// share a signing key, so neither triggers a refetch on its own.
final relayDirectoryUsersProvider =
    FutureProvider.autoDispose<List<DirectoryUser>>((ref) async {
      if (mockDmDirectoryEnabled) {
        return dmDirectoryPreviewUsers;
      }

      // Rebuild whenever the active relay/community configuration changes.
      ref.watch(relayConfigProvider);
      final session = ref.watch(relaySessionProvider.notifier);
      final currentPubkey = ref.watch(currentPubkeyProvider)?.toLowerCase();
      final directoryEvents = await session.queryRelay([
        const NostrFilter(kinds: [0], limit: 50, extensions: {'page': 1}),
      ]);
      var users = directoryUsersFromProfileEvents(directoryEvents);
      if (users.isNotEmpty) {
        return users
            .where((user) => user.pubkey.toLowerCase() != currentPubkey)
            .toList();
      }

      final membershipEvents = await session.fetchHistory(
        NostrFilters.relayMembers(),
      );
      final memberPubkeys = relayMemberPubkeysFromEvents(
        membershipEvents,
      ).where((pubkey) => pubkey != currentPubkey).toList();
      if (memberPubkeys.isEmpty) {
        return const [];
      }

      final profileEvents = await session.queryRelay([
        NostrFilters.profilesBatch(memberPubkeys),
      ]);
      final profilesByPubkey = {
        for (final event in profileEvents)
          event.pubkey.toLowerCase(): ProfileData.fromEvent(event),
      };
      users =
          [
            for (final pubkey in memberPubkeys)
              if (profilesByPubkey[pubkey] case final profile?)
                DirectoryUser(
                  pubkey: pubkey,
                  displayName: profile.displayName,
                  avatarUrl: profile.avatarUrl,
                  nip05Handle: profile.nip05,
                )
              else
                DirectoryUser(pubkey: pubkey),
          ]..sort((a, b) {
            final labelComparison = a.label.toLowerCase().compareTo(
              b.label.toLowerCase(),
            );
            return labelComparison != 0
                ? labelComparison
                : a.pubkey.compareTo(b.pubkey);
          });
      return users;
    });

/// Prefix-searches the active relay's kind:0 people directory.
///
/// autoDispose family: each distinct query would otherwise cache a provider
/// instance for the whole session (the mention search provider is autoDispose
/// for the same reason). The [relayConfigProvider] watch invalidates cached
/// results at the community boundary.
final relayDirectorySearchProvider = FutureProvider.autoDispose
    .family<List<DirectoryUser>, String>((ref, query) async {
      final trimmed = query.trim();
      if (mockDmDirectoryEnabled) {
        final normalizedQuery = trimmed.toLowerCase();
        return dmDirectoryPreviewUsers
            .where(
              (user) =>
                  user.label.toLowerCase().contains(normalizedQuery) ||
                  user.secondaryLabel.toLowerCase().contains(normalizedQuery),
            )
            .toList();
      }
      if (trimmed.isEmpty) {
        return ref.watch(relayDirectoryUsersProvider.future);
      }

      // Rebuild whenever the active relay/community configuration changes.
      ref.watch(relayConfigProvider);
      final session = ref.watch(relaySessionProvider.notifier);
      final currentPubkey = ref.watch(currentPubkeyProvider)?.toLowerCase();
      final events = await session.queryRelay([
        NostrFilters.searchUsers(trimmed, limit: 50),
      ]);
      return directoryUsersFromProfileEvents(
        events,
      ).where((user) => user.pubkey.toLowerCase() != currentPubkey).toList();
    });

/// Build [ChannelDetails] from a kind:39000 metadata event.
///
/// Exposed as a pure function so the mapping can be unit-tested without
/// Riverpod / WebSocket scaffolding. Make sure all fields parsed by
/// [ChannelData.fromEvent] that exist on [ChannelDetails] are propagated —
/// any omission silently drops state when [Channel.mergeDetails] is called.
@visibleForTesting
ChannelDetails channelDetailsFromEvent(NostrEvent event) {
  final data = ChannelData.fromEvent(event);
  final eventTime = DateTime.fromMillisecondsSinceEpoch(
    event.createdAt * 1000,
    isUtc: true,
  );
  return ChannelDetails(
    id: data.id,
    name: data.name,
    channelType: data.channelType,
    visibility: data.visibility,
    description: data.description,
    topic: data.topic,
    createdBy: event.pubkey,
    createdAt: eventTime,
    memberCount: 0,
    // Same archival-timestamp convention as `_channelFromMeta` — the event's
    // `createdAt` is when the relay republished the metadata. Without this,
    // `Channel.mergeDetails(details)` would clobber the archived state set
    // on the base channel and the detail view would show compose/manage
    // actions for expired/archived channels.
    archivedAt: data.isArchived ? eventTime : null,
    ttlSeconds: data.ttlSeconds,
    ttlDeadline: data.ttlDeadline,
  );
}

/// Single channel's metadata via kind:39000.
final channelDetailsProvider = FutureProvider.family<ChannelDetails, String>((
  ref,
  channelId,
) async {
  final session = ref.watch(relaySessionProvider.notifier);
  final events = await session.fetchHistory(
    NostrFilter(
      kinds: [39000],
      tags: {
        '#d': [channelId],
      },
      limit: 1,
    ),
  );
  if (events.isEmpty) {
    throw Exception('Channel not found: $channelId');
  }
  return channelDetailsFromEvent(events.first);
});

/// Channel members from kind:39002 NIP-29 members event.
final channelMembersProvider =
    FutureProvider.family<List<ChannelMember>, String>((ref, channelId) async {
      final session = ref.watch(relaySessionProvider.notifier);
      final events = await session.fetchHistory(
        NostrFilters.channelMembers(channelId),
      );
      if (events.isEmpty) return const [];
      final event = events.first;
      final joinedAt = DateTime.fromMillisecondsSinceEpoch(
        event.createdAt * 1000,
        isUtc: true,
      );
      return membersFromEvent(event)
          .map(
            (m) => ChannelMember(
              pubkey: m.pubkey,
              role: m.role,
              joinedAt: joinedAt,
            ),
          )
          .toList();
    });

/// Channel canvas (kind:40100 for the channel).
final channelCanvasProvider = FutureProvider.family<ChannelCanvas, String>((
  ref,
  channelId,
) async {
  final session = ref.watch(relaySessionProvider.notifier);
  final events = await session.fetchHistory(NostrFilters.canvas(channelId));
  if (events.isEmpty) {
    return const ChannelCanvas(
      content: null,
      updatedAt: null,
      authorPubkey: null,
    );
  }
  final event = events.first;
  return ChannelCanvas(
    content: event.content,
    updatedAt: DateTime.fromMillisecondsSinceEpoch(
      event.createdAt * 1000,
      isUtc: true,
    ),
    authorPubkey: event.pubkey,
  );
});

/// Channel-scoped kind:5 deletion tags. The `h` tag lets channel-scoped
/// subscriptions observe the delete; the `e` tag points at the target event.
@visibleForTesting
List<List<String>> buildDeleteMessageTags({
  required String channelId,
  required String eventId,
}) {
  return [
    ['h', channelId],
    ['e', eventId],
  ];
}

/// Builds the signed kind:9007 tags used by mobile channel creation.
List<List<String>> buildCreateChannelTags({
  required String channelId,
  required String name,
  required String channelType,
  required String visibility,
  String? description,
  int? ttlSeconds,
}) {
  if (ttlSeconds != null && ttlSeconds <= 0) {
    throw ArgumentError.value(ttlSeconds, 'ttlSeconds', 'must be positive');
  }

  return [
    ['h', channelId],
    ['name', name],
    ['visibility', visibility],
    ['channel_type', channelType],
    if (description case final about? when about.trim().isNotEmpty)
      ['about', about.trim()],
    if (ttlSeconds != null) ['ttl', ttlSeconds.toString()],
  ];
}

class ChannelActions {
  final Ref _ref;
  final RelaySessionNotifier _session;
  final SignedEventRelay _signedEventRelay;
  final String? _currentPubkey;

  ChannelActions({
    required Ref ref,
    required RelaySessionNotifier session,
    required SignedEventRelay signedEventRelay,
    required String? currentPubkey,
  }) : _ref = ref,
       _session = session,
       _signedEventRelay = signedEventRelay,
       _currentPubkey = currentPubkey;

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
    await _signedEventRelay.submit(kind: 9007, content: '', tags: tags);
    return _refreshChannelsAndRead(channelId);
  }

  /// Open (or create) a DM channel with the given pubkeys.
  ///
  /// This submits a kind:41010 command event; the relay responds with an OK
  /// message whose content carries `response:{...}` containing the new
  /// `channel_id`.
  Future<Channel> openDm({required List<String> pubkeys}) async {
    final result = await _signedEventRelay.submit(
      kind: 41010,
      content: '',
      tags: pubkeys.map((pk) => ['p', pk]).toList(),
    );
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
    for (final pubkey in normalizedPubkeys) {
      await _signedEventRelay.submit(
        kind: 9000,
        content: '',
        tags: [
          ['h', channelId],
          ['p', pubkey],
          ['role', normalizedRole],
        ],
      );
    }
    _ref.invalidate(channelMembersProvider(channelId));
  }

  Future<void> joinChannel(String channelId) async {
    await _signedEventRelay.submit(
      kind: 9021,
      content: '',
      tags: [
        ['h', channelId],
      ],
    );
    await _refreshChannelState(channelId);
  }

  Future<void> leaveChannel(String channelId) async {
    await _signedEventRelay.submit(
      kind: 9022,
      content: '',
      tags: [
        ['h', channelId],
      ],
    );
    await _refreshChannelState(channelId);
  }

  Future<void> setCanvas({
    required String channelId,
    required String content,
  }) async {
    await _signedEventRelay.submit(
      kind: 40100,
      content: content,
      tags: [
        ['h', channelId],
      ],
    );
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
    await _ref.read(channelsProvider.notifier).refresh();
    final channels = await _ref.read(channelsProvider.future);
    return channels.firstWhere(
      (channel) => channel.id == channelId,
      orElse: () =>
          throw Exception('Channel was created but is not visible yet'),
    );
  }

  Future<void> _refreshChannelState(String channelId) async {
    await _ref.read(channelsProvider.notifier).refresh();
    _ref.invalidate(channelDetailsProvider(channelId));
    _ref.invalidate(channelMembersProvider(channelId));
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
    await _signedEventRelay.submit(
      kind: 9000,
      content: '',
      tags: [
        ['h', channelId],
        ['p', pubkey.toLowerCase()],
        ['role', role],
      ],
    );
    _ref.invalidate(channelMembersProvider(channelId));
  }

  Future<void> removeMember({
    required String channelId,
    required String pubkey,
  }) async {
    await _signedEventRelay.submit(
      kind: 9001,
      content: '',
      tags: [
        ['h', channelId],
        ['p', pubkey.toLowerCase()],
      ],
    );
    _ref.invalidate(channelMembersProvider(channelId));
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
    await _signedEventRelay.submit(
      kind: EventKind.streamMessageEdit,
      content: content,
      tags: [
        ['h', channelId],
        ['e', eventId],
        ...mediaTags,
      ],
    );
  }

  Future<void> deleteMessage({
    required String channelId,
    required String eventId,
  }) async {
    await _signedEventRelay.submit(
      kind: EventKind.deletion,
      content: '',
      tags: buildDeleteMessageTags(channelId: channelId, eventId: eventId),
    );
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
  );
});
