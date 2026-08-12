import 'dart:convert';
import 'dart:ui' show Rect;

import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:share_plus/share_plus.dart';

import '../../shared/community/community_membership_provider.dart';
import '../../shared/relay/relay.dart';

/// The default lifetime of a newly minted community invite link.
const defaultCommunityInviteTtlSeconds = 3 * 24 * 60 * 60;

/// A labeled value shown in a community invite settings sheet.
@immutable
class CommunityInviteOption<T> {
  /// Creates an option with its user-facing [label] and submitted [value].
  const CommunityInviteOption({required this.label, required this.value});

  /// The text presented for this option.
  final String label;

  /// The value applied when this option is selected.
  final T value;
}

/// Supported community invite-link lifetimes.
const communityInviteTtlOptions = [
  CommunityInviteOption(label: '1 day', value: 24 * 60 * 60),
  CommunityInviteOption(label: '3 days', value: 3 * 24 * 60 * 60),
  CommunityInviteOption(label: '7 days', value: 7 * 24 * 60 * 60),
  CommunityInviteOption(label: '30 days', value: 30 * 24 * 60 * 60),
];

/// Supported usage limits for a community invite link.
const communityInviteMaxUseOptions = <CommunityInviteOption<int?>>[
  CommunityInviteOption(label: 'No limit', value: null),
  CommunityInviteOption(label: '1 use', value: 1),
  CommunityInviteOption(label: '3 uses', value: 3),
  CommunityInviteOption(label: '5 uses', value: 5),
  CommunityInviteOption(label: '10 uses', value: 10),
  CommunityInviteOption(label: '25 uses', value: 25),
];

/// A community invite link minted by the active relay.
@immutable
class MintedCommunityInvite {
  /// Creates a parsed invite-link response.
  const MintedCommunityInvite({
    required this.code,
    required this.expiresAt,
    required this.url,
    required this.maxUses,
    required this.usesRemaining,
  });

  /// The relay-issued invite code.
  final String code;

  /// The invite's expiration time in Unix seconds.
  final int expiresAt;

  /// The complete URL that recipients can open.
  final String url;

  /// The total permitted uses, or null when unlimited.
  final int? maxUses;

  /// The number of uses still available, or null when unlimited.
  final int? usesRemaining;

  /// Parses a relay invite response.
  factory MintedCommunityInvite.fromJson(Map<String, dynamic> json) {
    return MintedCommunityInvite(
      code: json['code'] as String,
      expiresAt: json['expires_at'] as int,
      url: json['url'] as String,
      maxUses: json['max_uses'] as int?,
      usesRemaining: json['uses_remaining'] as int?,
    );
  }
}

/// A relay directory identity that can be invited to a community.
@immutable
class CommunityInviteDirectoryUser {
  /// Creates a directory identity for [pubkey] and optional profile metadata.
  const CommunityInviteDirectoryUser({
    required this.pubkey,
    this.displayName,
    this.avatarUrl,
    this.nip05Handle,
  });

  /// The identity's lowercase hexadecimal Nostr public key.
  final String pubkey;

  /// The profile's preferred display name, when published.
  final String? displayName;

  /// The profile image URL, when published.
  final String? avatarUrl;

  /// The profile's NIP-05 identifier, when published.
  final String? nip05Handle;

  /// The best available human-readable identity label.
  String get label {
    final display = displayName?.trim();
    if (display != null && display.isNotEmpty) return display;
    final nip05 = nip05Handle?.trim();
    if (nip05 != null && nip05.isNotEmpty) return nip05;
    return shortCommunityInvitePubkey(pubkey);
  }

  /// A supporting identity label distinct from [label].
  String get secondaryLabel {
    final nip05 = nip05Handle?.trim();
    if (nip05 != null && nip05.isNotEmpty && nip05 != label) return nip05;
    return pubkey.length > 16 ? '${pubkey.substring(0, 16)}…' : pubkey;
  }

  /// The uppercase first character of [label], or `?` when unavailable.
  String get initial => label.isEmpty ? '?' : label[0].toUpperCase();
}

/// Abbreviates a hexadecimal public key for compact display.
String shortCommunityInvitePubkey(String pubkey) =>
    pubkey.length > 8 ? '${pubkey.substring(0, 8)}…' : pubkey;

/// Encodes and abbreviates a hexadecimal public key as an npub.
String shortCommunityInviteNpub(String pubkey) {
  try {
    final npub = nostr.Nip19.encode(
      prefix: nostr.Nip19Prefix.npub,
      data: pubkey,
    );
    return npub.length > 20
        ? '${npub.substring(0, 12)}…${npub.substring(npub.length - 6)}'
        : npub;
  } catch (_) {
    return shortCommunityInvitePubkey(pubkey);
  }
}

/// Parses a hexadecimal public key or npub, returning lowercase hex.
String? parseCommunityInvitePubkey(String value) {
  final normalized = value.trim();
  final hexPattern = RegExp(r'^[0-9a-fA-F]{64}$');
  if (hexPattern.hasMatch(normalized)) return normalized.toLowerCase();
  try {
    final decoded = nostr.Nip19.decode(payload: normalized);
    if (decoded.prefix != nostr.Nip19Prefix.npub ||
        !hexPattern.hasMatch(decoded.data)) {
      return null;
    }
    return decoded.data.toLowerCase();
  } catch (_) {
    return null;
  }
}

/// Builds the Nostr tags for a kind:9030 community member invitation.
@visibleForTesting
List<List<String>> buildCommunityMemberInviteTags({
  required String pubkey,
  required CommunityMemberRole role,
}) => [
  ['p', pubkey.trim().toLowerCase()],
  ['role', role.name],
];

/// Creates invite links and submits direct community member invitations.
abstract class CommunityInviteActions {
  /// Mints a community invite link with the requested limits.
  Future<MintedCommunityInvite> mintInvite({
    required int ttlSeconds,
    required int? maxUses,
  });

  /// Invites each public key to the active community with [role].
  Future<void> inviteMembers({
    required Iterable<String> pubkeys,
    required CommunityMemberRole role,
  });
}

/// Relay-backed implementation of [CommunityInviteActions].
class RelayCommunityInviteActions implements CommunityInviteActions {
  /// Creates invite actions bound to the active relay and signing session.
  RelayCommunityInviteActions({
    required http.Client httpClient,
    required String baseUrl,
    required String? nsec,
    required SignedEventRelay signedEventRelay,
    required bool Function() isCommunityActive,
  }) : _httpClient = httpClient,
       _baseUrl = baseUrl,
       _nsec = nsec,
       _signedEventRelay = signedEventRelay,
       _isCommunityActive = isCommunityActive;

  final http.Client _httpClient;
  final String _baseUrl;
  final String? _nsec;
  final SignedEventRelay _signedEventRelay;
  final bool Function() _isCommunityActive;

  void _ensureCommunityActive() {
    if (!_isCommunityActive()) {
      throw StateError('Invite cancelled because the active community changed');
    }
  }

  @override
  Future<MintedCommunityInvite> mintInvite({
    required int ttlSeconds,
    required int? maxUses,
  }) async {
    _ensureCommunityActive();
    final url = Uri.parse(_baseUrl).resolve('/api/invites').toString();
    final body = <String, Object>{'ttl_secs': ttlSeconds};
    if (maxUses != null) body['max_uses'] = maxUses;
    final bodyBytes = utf8.encode(jsonEncode(body));
    final response = await _httpClient
        .post(
          Uri.parse(url),
          headers: {
            'Authorization': buildNip98AuthHeader(
              method: 'POST',
              url: url,
              bodyBytes: bodyBytes,
              nsec: _nsec,
            ),
            'Content-Type': 'application/json',
          },
          body: bodyBytes,
        )
        .timeout(const Duration(seconds: 15));
    _ensureCommunityActive();

    final dynamic decoded;
    try {
      decoded = jsonDecode(response.body);
    } on FormatException {
      throw Exception('Relay returned an invalid invite response');
    }
    if (response.statusCode < 200 || response.statusCode >= 300) {
      final rawMessage = decoded is Map<String, dynamic>
          ? decoded['error']
          : null;
      final message = rawMessage is String ? rawMessage : null;
      throw Exception(message ?? 'HTTP ${response.statusCode}');
    }
    if (decoded is! Map<String, dynamic>) {
      throw Exception('Relay returned an invalid invite response');
    }
    return MintedCommunityInvite.fromJson(decoded);
  }

  @override
  Future<void> inviteMembers({
    required Iterable<String> pubkeys,
    required CommunityMemberRole role,
  }) async {
    final normalizedPubkeys = <String>{};
    for (final pubkey in pubkeys) {
      final parsed = parseCommunityInvitePubkey(pubkey);
      if (parsed != null) normalizedPubkeys.add(parsed);
    }
    for (final pubkey in normalizedPubkeys) {
      _ensureCommunityActive();
      await _signedEventRelay.submit(
        kind: EventKind.relayAdminAddMember,
        content: '',
        tags: buildCommunityMemberInviteTags(pubkey: pubkey, role: role),
      );
    }
    _ensureCommunityActive();
  }
}

/// Supplies the HTTP client used to mint community invite links.
final communityInviteHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

/// Supplies invite operations bound to the current community session.
final communityInviteActionsProvider = Provider<CommunityInviteActions>((ref) {
  final config = ref.watch(relayConfigProvider);
  final session = ref.read(relaySessionProvider.notifier);
  return RelayCommunityInviteActions(
    httpClient: ref.watch(communityInviteHttpClientProvider),
    baseUrl: config.baseUrl,
    nsec: config.nsec,
    signedEventRelay: SignedEventRelay(session: session, nsec: config.nsec),
    isCommunityActive: () {
      final current = ref.read(relayConfigProvider);
      return current.baseUrl == config.baseUrl && current.nsec == config.nsec;
    },
  );
});

List<CommunityInviteDirectoryUser> _directoryUsersFromEvents(
  List<NostrEvent> events,
) {
  final latestByPubkey = <String, NostrEvent>{};
  for (final event in events) {
    if (event.kind != 0) continue;
    final pubkey = event.pubkey.toLowerCase();
    final current = latestByPubkey[pubkey];
    if (current == null || event.createdAt > current.createdAt) {
      latestByPubkey[pubkey] = event;
    }
  }
  final users = [
    for (final event in latestByPubkey.values)
      if (ProfileData.fromEvent(event) case final profile)
        CommunityInviteDirectoryUser(
          pubkey: profile.pubkey.toLowerCase(),
          displayName: profile.displayName,
          avatarUrl: profile.avatarUrl,
          nip05Handle: profile.nip05,
        ),
  ];
  users.sort((a, b) {
    final byLabel = a.label.toLowerCase().compareTo(b.label.toLowerCase());
    return byLabel != 0 ? byLabel : a.pubkey.compareTo(b.pubkey);
  });
  return users;
}

/// Resolves a pasted pubkey to its published profile on the active relay.
///
/// A valid npub already identifies an exact public key, so missing kind:0
/// metadata falls back to a pubkey-only person instead of blocking the invite.
/// Relay failures still surface as errors and malformed inputs return null.
final communityInviteProfileProvider = FutureProvider.autoDispose
    .family<CommunityInviteDirectoryUser?, String>((ref, pubkey) async {
      final normalized = parseCommunityInvitePubkey(pubkey);
      if (normalized == null) return null;
      ref.watch(relayConfigProvider);
      final events = await ref.watch(relaySessionProvider.notifier).queryRelay([
        NostrFilters.profile(normalized),
      ]);
      for (final user in _directoryUsersFromEvents(events)) {
        if (user.pubkey == normalized) return user;
      }
      return CommunityInviteDirectoryUser(pubkey: normalized);
    });

/// Lists inviteable relay identities, excluding the signed-in user.
final communityInviteDirectoryProvider =
    FutureProvider.autoDispose<List<CommunityInviteDirectoryUser>>((ref) async {
      ref.watch(relayConfigProvider);
      final currentPubkey = ref.watch(myPubkeyProvider)?.toLowerCase();
      final events = await ref.watch(relaySessionProvider.notifier).queryRelay([
        const NostrFilter(kinds: [0], limit: 50, extensions: {'page': 1}),
      ]);
      final directoryUsers = _directoryUsersFromEvents(
        events,
      ).where((user) => user.pubkey != currentPubkey).toList();
      if (directoryUsers.isNotEmpty) return directoryUsers;

      // Match the existing mobile person picker: older relays may not support
      // a paged kind:0 directory, so fall back to the membership snapshot and
      // hydrate whatever profiles are available.
      final membership = await ref.watch(communityMembershipProvider.future);
      final memberPubkeys = membership.pubkeys
          .where((pubkey) => pubkey != currentPubkey)
          .toList();
      if (memberPubkeys.isEmpty) return const [];
      final profileEvents = await ref
          .watch(relaySessionProvider.notifier)
          .queryRelay([NostrFilters.profilesBatch(memberPubkeys)]);
      final profilesByPubkey = {
        for (final user in _directoryUsersFromEvents(profileEvents))
          user.pubkey: user,
      };
      final fallbackUsers = [
        for (final pubkey in memberPubkeys)
          profilesByPubkey[pubkey] ??
              CommunityInviteDirectoryUser(pubkey: pubkey),
      ];
      fallbackUsers.sort((a, b) {
        final byLabel = a.label.toLowerCase().compareTo(b.label.toLowerCase());
        return byLabel != 0 ? byLabel : a.pubkey.compareTo(b.pubkey);
      });
      return fallbackUsers;
    });

/// Searches the active relay for inviteable identities matching a query.
final communityInviteDirectorySearchProvider = FutureProvider.autoDispose
    .family<List<CommunityInviteDirectoryUser>, String>((ref, query) async {
      final trimmed = query.trim();
      if (trimmed.isEmpty) {
        return ref.watch(communityInviteDirectoryProvider.future);
      }
      ref.watch(relayConfigProvider);
      final currentPubkey = ref.watch(myPubkeyProvider)?.toLowerCase();
      final events = await ref.watch(relaySessionProvider.notifier).queryRelay([
        NostrFilters.searchUsers(trimmed, limit: 50),
      ]);
      return _directoryUsersFromEvents(
        events,
      ).where((user) => user.pubkey != currentPubkey).toList();
    });

/// Shares [inviteUrl] from an optional platform anchor rectangle.
typedef ShareCommunityInvite =
    Future<void> Function(String inviteUrl, Rect? sharePositionOrigin);

/// Supplies the native share-sheet action for community invite links.
final shareCommunityInviteProvider = Provider<ShareCommunityInvite>((ref) {
  return (inviteUrl, sharePositionOrigin) async {
    await SharePlus.instance.share(
      ShareParams(text: inviteUrl, sharePositionOrigin: sharePositionOrigin),
    );
  };
});
