import 'dart:convert';
import 'dart:isolate';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/relay/relay.dart';
import 'channel_management_provider.dart';

/// Kind of the managed-agent record that Desktop publishes, signed by the
/// owner, for every agent it runs. Its `d` tag is the agent's pubkey.
const managedAgentRecordKind = 30177;

/// Which members of a Huddle channel are agents.
///
/// The channel role `bot` is the first signal. The relay stores it only for
/// keys with a recorded NIP-OA owner, so an agent without an owner
/// attestation joins as `member`. Such an agent is still an agent here when
/// a trusted author published a kind:30177 managed-agent record for it.
///
/// Trusted record authors are the current user and the Huddle channel's
/// owners. A record from anyone else is only that author's claim. This is a
/// Huddle compatibility policy, not identity proof: it mints no owner. It
/// lets an owner mark a member as an agent for the participant list, labels,
/// the "preparing a response" state, and the last-person count that decides
/// whether leaving ends and archives the Huddle. An owner already had that
/// power when the client chose the `bot` role. Desktop uses the same rule
/// for its Huddle roster (block/buzz#8544).
///
/// The result holds only current members. Until the records first load,
/// only the `bot` role counts. While the records reload after a membership
/// change, or if that reload fails, the last loaded records still apply to
/// the members who remain; a failed read never removes a `bot` agent.
final huddleAgentPubkeysProvider = Provider.autoDispose
    .family<Set<String>, String>((ref, channelId) {
      final members =
          ref.watch(channelMembersProvider(channelId)).value ??
          const <ChannelMember>[];
      final recordAgents =
          ref.watch(huddleManagedAgentRecordPubkeysProvider(channelId)).value ??
          const <String>{};
      return Set.unmodifiable({
        for (final member in members)
          if (member.isBot ||
              recordAgents.contains(member.pubkey.trim().toLowerCase()))
            member.pubkey.trim().toLowerCase(),
      });
    });

final huddleManagedAgentRecordPubkeysProvider = FutureProvider.autoDispose
    .family<Set<String>, String>((ref, channelId) async {
      final connected = ref.watch(
        relaySessionProvider.select(
          (state) => state.status == SessionStatus.connected,
        ),
      );
      final viewer = ref.watch(myPubkeyProvider);
      final members = await ref.watch(channelMembersProvider(channelId).future);
      if (!connected) return const {};
      try {
        return await fetchTrustedManagedAgentPubkeys(
          ref.read(relaySessionProvider.notifier),
          members: [
            for (final member in members)
              MemberEntry(pubkey: member.pubkey, role: member.role),
          ],
          viewerPubkey: viewer,
        );
      } catch (error) {
        debugPrint('[HuddleAgents] managed-agent query failed: $error');
        rethrow;
      }
    });

/// Members of a Huddle channel, other than `bot` members, that a trusted
/// author recorded as a managed agent. Throws when the records cannot be read.
Future<Set<String>> fetchTrustedManagedAgentPubkeys(
  RelaySessionNotifier session, {
  required Iterable<MemberEntry> members,
  required String? viewerPubkey,
}) async {
  final authors = trustedManagedAgentRecordAuthors(
    members: members,
    viewerPubkey: viewerPubkey,
  );
  final candidates = managedAgentRecordCandidates(members);
  if (authors.isEmpty || candidates.isEmpty) return const {};
  final events = await session.fetchHistory(
    NostrFilter(
      kinds: const [managedAgentRecordKind],
      authors: authors,
      tags: {'#d': candidates},
      limit: authors.length * candidates.length,
    ),
  );
  // Signature checks are CPU work; keep them off the UI isolate.
  return Isolate.run(
    () => managedAgentPubkeysFromRecords(
      events: events,
      trustedAuthors: authors,
      candidates: candidates,
    ),
  );
}

/// The current user and the channel's owners, normalized and deduplicated.
@visibleForTesting
List<String> trustedManagedAgentRecordAuthors({
  required Iterable<MemberEntry> members,
  required String? viewerPubkey,
}) {
  final authors = <String>{};
  final viewer = viewerPubkey?.trim().toLowerCase();
  if (viewer != null && viewer.isNotEmpty) authors.add(viewer);
  for (final member in members) {
    if (member.role == 'owner') authors.add(member.pubkey.trim().toLowerCase());
  }
  return List.unmodifiable(authors);
}

/// Members without the `bot` role. Only these need a managed-agent record.
@visibleForTesting
List<String> managedAgentRecordCandidates(Iterable<MemberEntry> members) =>
    List.unmodifiable({
      for (final member in members)
        if (member.role != 'bot') member.pubkey.trim().toLowerCase(),
    });

/// Candidates named by a validly signed managed-agent record from a trusted
/// author.
@visibleForTesting
Set<String> managedAgentPubkeysFromRecords({
  required Iterable<NostrEvent> events,
  required Iterable<String> trustedAuthors,
  required Iterable<String> candidates,
}) {
  final trusted = {for (final author in trustedAuthors) author.toLowerCase()};
  final members = {for (final pubkey in candidates) pubkey.toLowerCase()};
  final agents = <String>{};
  for (final event in events) {
    if (event.kind != managedAgentRecordKind) continue;
    if (!trusted.contains(event.pubkey.toLowerCase())) continue;
    final agent = _firstTagValue(event, 'd')?.toLowerCase();
    if (agent == null || !members.contains(agent)) continue;
    if (!_hasValidSignature(event)) continue;
    agents.add(agent);
  }
  return Set.unmodifiable(agents);
}

String? _firstTagValue(NostrEvent event, String name) {
  for (final tag in event.tags) {
    if (tag.length >= 2 && tag[0] == name) return tag[1];
  }
  return null;
}

/// `nostr.Event.fromJson` checks the event id and the signature, and throws
/// when either is wrong.
bool _hasValidSignature(NostrEvent event) {
  try {
    nostr.Event.fromJson(jsonEncode(event.toJson()));
    return true;
  } catch (_) {
    return false;
  }
}
