import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../relay/relay.dart';

enum CommunityMemberRole { owner, admin, member }

@immutable
class CommunityMember {
  const CommunityMember({required this.pubkey, required this.role});

  final String pubkey;
  final CommunityMemberRole role;
}

@immutable
class CommunityMembershipSnapshot {
  const CommunityMembershipSnapshot({
    required this.snapshotFound,
    required this.members,
  });

  final bool snapshotFound;
  final List<CommunityMember> members;

  CommunityMemberRole? roleFor(String? pubkey) {
    final normalized = pubkey?.trim().toLowerCase();
    if (normalized == null || normalized.isEmpty) return null;
    for (final member in members) {
      if (member.pubkey == normalized) return member.role;
    }
    return null;
  }

  Set<String> get pubkeys => {for (final member in members) member.pubkey};
}

CommunityMemberRole _communityMemberRole(String? value) => switch (value) {
  'owner' => CommunityMemberRole.owner,
  'admin' => CommunityMemberRole.admin,
  _ => CommunityMemberRole.member,
};

/// Parses the current kind:13534 membership snapshot.
///
/// Buzz emits `["member", pubkey, role]`. Older NIP-29-compatible relays may
/// use `["p", pubkey, relay, role]`, so mobile accepts both forms just like
/// desktop does.
@visibleForTesting
CommunityMembershipSnapshot communityMembershipFromEvents(
  List<NostrEvent> events,
) {
  if (events.isEmpty) {
    return const CommunityMembershipSnapshot(snapshotFound: false, members: []);
  }

  final event = events.reduce(
    (latest, candidate) =>
        candidate.createdAt > latest.createdAt ? candidate : latest,
  );
  final members = <CommunityMember>[];
  final seen = <String>{};
  final pubkeyPattern = RegExp(r'^[0-9a-f]{64}$');

  for (final tag in event.tags) {
    if (tag.length < 2 || (tag[0] != 'member' && tag[0] != 'p')) continue;
    final pubkey = tag[1].trim().toLowerCase();
    if (!pubkeyPattern.hasMatch(pubkey) || !seen.add(pubkey)) continue;
    final role = tag[0] == 'member'
        ? (tag.length >= 3 ? tag[2] : null)
        : (tag.length >= 4 ? tag[3] : null);
    members.add(
      CommunityMember(pubkey: pubkey, role: _communityMemberRole(role)),
    );
  }

  return CommunityMembershipSnapshot(snapshotFound: true, members: members);
}

/// Relay membership for the active community.
///
/// The HTTP query resolves from the first response rather than waiting for a
/// WebSocket EOSE frame, and watching [relayConfigProvider] makes the result
/// community-scoped.
final communityMembershipProvider =
    FutureProvider.autoDispose<CommunityMembershipSnapshot>((ref) async {
      ref.watch(relayConfigProvider);
      final session = ref.watch(relaySessionProvider.notifier);
      final events = await session.queryRelay([NostrFilters.relayMembers()]);
      return communityMembershipFromEvents(events);
    });

final currentCommunityRoleProvider = Provider<AsyncValue<CommunityMemberRole?>>(
  (ref) {
    final pubkey = ref.watch(myPubkeyProvider);
    return ref
        .watch(communityMembershipProvider)
        .whenData((snapshot) => snapshot.roleFor(pubkey));
  },
);

bool canManageCommunityInvites(CommunityMemberRole? role) =>
    role == CommunityMemberRole.owner || role == CommunityMemberRole.admin;
