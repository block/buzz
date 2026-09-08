import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/auth/auth.dart';
import '../../shared/custom_emoji/custom_emoji.dart';
import '../../shared/custom_emoji/custom_emoji_provider.dart';
import '../../shared/crypto/nip_oa.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/relay/relay.dart';
import '../profile/profile_provider.dart';
import 'channel.dart';
import 'channel_metadata_updates.dart';
import 'channels_provider.dart';

export 'channel_metadata_updates.dart';

part 'channel_huddle_actions.dart';
part 'channel_management_actions.dart';

String _relayErrorMessage(Object error) =>
    error.toString().replaceFirst('Exception: ', '');

/// Raised when one or more kind:9000 adds were rejected, keyed by pubkey.
///
/// Callers surface [message] to the user — a relay rejection here (e.g. a plain
/// member trying to add someone to a private channel) is a real outcome, not a
/// crash to swallow.
@immutable
class AddMembersException implements Exception {
  final Map<String, String> failures;

  const AddMembersException(this.failures);

  String get message => failures.entries
      .map(
        (entry) =>
            '${entry.key.length > 8 ? '${entry.key.substring(0, 8)}…' : entry.key}: ${entry.value}',
      )
      .join('; ');

  @override
  String toString() => 'AddMembersException($message)';
}

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

String _channelMemberSnapshotKey({
  required String relayBaseUrl,
  required String? pubkey,
  required String channelId,
}) =>
    '${relayBaseUrl.toLowerCase()}::${pubkey?.toLowerCase() ?? 'anon'}::$channelId';

class _ChannelMembersSnapshotCache {
  final _membersByKey = <String, List<ChannelMember>>{};

  List<ChannelMember>? read({
    required String relayBaseUrl,
    required String? pubkey,
    required String channelId,
  }) =>
      _membersByKey[_channelMemberSnapshotKey(
        relayBaseUrl: relayBaseUrl,
        pubkey: pubkey,
        channelId: channelId,
      )];

  void write({
    required String relayBaseUrl,
    required String? pubkey,
    required String channelId,
    required List<ChannelMember> members,
  }) {
    _membersByKey[_channelMemberSnapshotKey(
      relayBaseUrl: relayBaseUrl,
      pubkey: pubkey,
      channelId: channelId,
    )] = List.unmodifiable(
      members,
    );
  }
}

final _channelMembersSnapshotCacheProvider =
    Provider<_ChannelMembersSnapshotCache>((ref) {
      return _ChannelMembersSnapshotCache();
    });

/// Uses the relay-backed member list when connected, but keeps the channel
/// snapshot visible while a reconnect temporarily interrupts the refresh.
///
/// The provider retains its current value while disconnected and also keeps a
/// relay/account/channel-scoped snapshot for consumers that mount during the
/// reconnect window. An empty member list is authoritative only after a
/// connected fetch completes.
List<ChannelMember> channelMembersForAutocomplete({
  required AsyncValue<List<ChannelMember>> membersAsync,
  required SessionStatus sessionStatus,
  required List<ChannelMember> cachedMembers,
}) {
  final loadedMembers = membersAsync.asData?.value;
  if (loadedMembers == null) return cachedMembers;
  if (sessionStatus != SessionStatus.connected && loadedMembers.isEmpty) {
    return cachedMembers;
  }
  return loadedMembers;
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
  final bool isAgent;

  const DirectoryUser({
    required this.pubkey,
    this.displayName,
    this.avatarUrl,
    this.nip05Handle,
    this.isAgent = false,
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
          isAgent: verifiedOaOwnerPubkey(event.tags, event.pubkey) != null,
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
                  isAgent:
                      verifiedOaOwnerPubkey(
                        profileEvents
                            .firstWhere(
                              (event) => event.pubkey.toLowerCase() == pubkey,
                            )
                            .tags,
                        pubkey,
                      ) !=
                      null,
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
final channelMembersProvider = FutureProvider.autoDispose
    .family<List<ChannelMember>, String>((ref, channelId) async {
      ref.watch(
        channelMembershipUpdateProvider(
          channelId,
        ).select((update) => update.version),
      );
      final relayBaseUrl = ref.watch(relayConfigProvider).baseUrl;
      final pubkey = ref.watch(myPubkeyProvider)?.toLowerCase();
      final snapshotCache = ref.read(_channelMembersSnapshotCacheProvider);
      // Re-fetch only after reconnect completes. During the disconnected
      // interval this provider has no session dependency, so its current value
      // remains visible to every consumer rather than becoming AsyncData([]).
      ref.listen(relaySessionProvider, (previous, next) {
        if (next.status == SessionStatus.connected &&
            previous?.status != SessionStatus.connected) {
          ref.invalidateSelf();
        }
      });
      final sessionState = ref.read(relaySessionProvider);
      if (sessionState.status != SessionStatus.connected) {
        final cachedMembers = snapshotCache.read(
          relayBaseUrl: relayBaseUrl,
          pubkey: pubkey,
          channelId: channelId,
        );
        if (cachedMembers != null) return cachedMembers;
        final channelListMembers = ref
            .read(channelsProvider.notifier)
            .cachedMembersForChannel(channelId);
        if (channelListMembers.isNotEmpty) {
          snapshotCache.write(
            relayBaseUrl: relayBaseUrl,
            pubkey: pubkey,
            channelId: channelId,
            members: channelListMembers,
          );
          return channelListMembers;
        }
        return const [];
      }
      final session = ref.read(relaySessionProvider.notifier);
      final events = await session.fetchHistory(
        NostrFilters.channelMembers(channelId),
      );
      if (events.isEmpty) {
        snapshotCache.write(
          relayBaseUrl: relayBaseUrl,
          pubkey: pubkey,
          channelId: channelId,
          members: const [],
        );
        return const [];
      }
      final event = events.first;
      final joinedAt = DateTime.fromMillisecondsSinceEpoch(
        event.createdAt * 1000,
        isUtc: true,
      );
      final members = membersFromEvent(event)
          .map(
            (m) => ChannelMember(
              pubkey: m.pubkey,
              role: m.role,
              joinedAt: joinedAt,
            ),
          )
          .toList();
      snapshotCache.write(
        relayBaseUrl: relayBaseUrl,
        pubkey: pubkey,
        channelId: channelId,
        members: members,
      );
      return members;
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
