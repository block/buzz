import 'dart:convert';
import 'dart:ui' show Rect;

import 'package:flutter/foundation.dart';
import 'package:http/http.dart' as http;
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:share_plus/share_plus.dart';

import '../../shared/community/community_membership_provider.dart';
import '../../shared/relay/relay.dart';

const defaultCommunityInviteTtlSeconds = 3 * 24 * 60 * 60;

@immutable
class CommunityInviteOption<T> {
  const CommunityInviteOption({required this.label, required this.value});

  final String label;
  final T value;
}

const communityInviteTtlOptions = [
  CommunityInviteOption(label: '1 day', value: 24 * 60 * 60),
  CommunityInviteOption(label: '3 days', value: 3 * 24 * 60 * 60),
  CommunityInviteOption(label: '7 days', value: 7 * 24 * 60 * 60),
  CommunityInviteOption(label: '30 days', value: 30 * 24 * 60 * 60),
];

const communityInviteMaxUseOptions = <CommunityInviteOption<int?>>[
  CommunityInviteOption(label: 'No limit', value: null),
  CommunityInviteOption(label: '1 use', value: 1),
  CommunityInviteOption(label: '3 uses', value: 3),
  CommunityInviteOption(label: '5 uses', value: 5),
  CommunityInviteOption(label: '10 uses', value: 10),
  CommunityInviteOption(label: '25 uses', value: 25),
];

@immutable
class MintedCommunityInvite {
  const MintedCommunityInvite({
    required this.code,
    required this.expiresAt,
    required this.url,
    required this.maxUses,
    required this.usesRemaining,
  });

  final String code;
  final int expiresAt;
  final String url;
  final int? maxUses;
  final int? usesRemaining;

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

@immutable
class CommunityInviteDirectoryUser {
  const CommunityInviteDirectoryUser({
    required this.pubkey,
    this.displayName,
    this.avatarUrl,
    this.nip05Handle,
  });

  final String pubkey;
  final String? displayName;
  final String? avatarUrl;
  final String? nip05Handle;

  String get label {
    final display = displayName?.trim();
    if (display != null && display.isNotEmpty) return display;
    final nip05 = nip05Handle?.trim();
    if (nip05 != null && nip05.isNotEmpty) return nip05;
    return shortCommunityInvitePubkey(pubkey);
  }

  String get secondaryLabel {
    final nip05 = nip05Handle?.trim();
    if (nip05 != null && nip05.isNotEmpty && nip05 != label) return nip05;
    return pubkey.length > 16 ? '${pubkey.substring(0, 16)}…' : pubkey;
  }

  String get initial => label.isEmpty ? '?' : label[0].toUpperCase();
}

String shortCommunityInvitePubkey(String pubkey) =>
    pubkey.length > 8 ? '${pubkey.substring(0, 8)}…' : pubkey;

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

@visibleForTesting
List<List<String>> buildCommunityMemberInviteTags({
  required String pubkey,
  required CommunityMemberRole role,
}) => [
  ['p', pubkey.trim().toLowerCase()],
  ['role', role.name],
];

abstract class CommunityInviteActions {
  Future<MintedCommunityInvite> mintInvite({
    required int ttlSeconds,
    required int? maxUses,
  });

  Future<void> inviteMembers({
    required Iterable<String> pubkeys,
    required CommunityMemberRole role,
  });
}

class RelayCommunityInviteActions implements CommunityInviteActions {
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

final communityInviteHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

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

typedef ShareCommunityInvite =
    Future<void> Function(String inviteUrl, Rect? sharePositionOrigin);

final shareCommunityInviteProvider = Provider<ShareCommunityInvite>((ref) {
  return (inviteUrl, sharePositionOrigin) async {
    await SharePlus.instance.share(
      ShareParams(text: inviteUrl, sharePositionOrigin: sharePositionOrigin),
    );
  };
});
