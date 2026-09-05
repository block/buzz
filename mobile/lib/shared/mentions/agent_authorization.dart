part of 'agent_identity_provider.dart';

/// Fresh exact-key policy and relay-signed membership. Directory entries are
/// hints; only this read may authorize agent mentions at a destination.
Future<List<AgentDirectoryEntry>> readAgentAuthorization(
  RelaySessionNotifier session,
  Set<String> requestedKeys, {
  required String? viewer,
  String? channelId,
  required bool Function() isCurrent,
}) async {
  void check() {
    if (!isCurrent()) throw StateError('Agent authorization scope changed');
  }

  check();
  if (requestedKeys.isEmpty) return [];
  if (viewer == null ||
      requestedKeys.length > 1000 ||
      requestedKeys.any((key) => !RegExp(r'^[0-9a-f]{64}$').hasMatch(key))) {
    throw StateError('Invalid agent authorization request');
  }
  final authority = await session.fetchRelaySelf();
  check();
  final membership = await _membershipPages(
    session,
    authority,
    viewer,
    channelId,
    check,
  );
  check();
  final runtime = await _queryAgentFilters(session, [
    for (final key in requestedKeys)
      NostrFilter(kinds: const [10100], authors: [key], limit: 1),
  ], checkCurrent: check);
  check();
  // Guard every query stage, not merely the result: a mutable session must not
  // continue an old account/community request against the newly selected relay.
  final agents = await resolveAgentPolicies(
    session,
    runtime.where((e) => requestedKeys.contains(e.pubkey)).toList(),
    requestedKeys: requestedKeys,
    checkCurrent: check,
  );
  check();
  final latest = <String, NostrEvent>{};
  for (final event in membership) {
    if (event.kind != 39002 || event.pubkey != authority) {
      continue;
    }
    final destination = event.getTagValue('d');
    if (destination == null ||
        (channelId != null && destination != channelId)) {
      continue;
    }
    final previous = latest[destination];
    if (previous == null || _newer(event, previous)) {
      latest[destination] = event;
    }
  }
  final result = <AgentDirectoryEntry>[];
  for (final agent in agents) {
    final owned = agent.ownerPubkey == viewer;
    final channels = <String>[];
    for (final entry in latest.entries) {
      if (!verifySignedEvent(entry.value) ||
          entry.value.tags
                  .where((tag) => tag.isNotEmpty && tag[0] == 'd')
                  .length !=
              1) {
        continue;
      }
      final people = entry.value.tags.where(
        (tag) => tag.length >= 2 && tag[0] == 'p',
      );
      if (!people.any((tag) => tag[1] == viewer)) continue;
      if (people.any(
        (tag) =>
            tag[1] == agent.pubkey &&
            (owned || (tag.length >= 4 && tag[3] == 'bot')),
      )) {
        channels.add(entry.key);
      }
    }
    if (!owned && channels.isEmpty) continue;
    result.add(
      AgentDirectoryEntry(
        pubkey: agent.pubkey,
        ownerPubkey: agent.ownerPubkey,
        displayName: agent.displayName,
        respondTo: agent.respondTo,
        respondToAllowlist: agent.respondToAllowlist,
        channelIds: channels,
      ),
    );
  }
  return result;
}

Future<List<NostrEvent>> _membershipPages(
  RelaySessionNotifier session,
  String authority,
  String viewer,
  String? channelId,
  void Function() check,
) async {
  final result = <NostrEvent>[];
  NostrEvent? cursor;
  for (var page = 0; page < 200; page++) {
    check();
    final events = await session.queryRelay([
      NostrFilter(
        kinds: const [39002],
        authors: [authority],
        tags: {
          if (channelId == null) '#p': [viewer],
          if (channelId != null) '#d': [channelId],
        },
        limit: 500,
        until: cursor?.createdAt,
        extensions: {if (cursor != null) 'before_id': cursor.id},
      ),
    ]);
    check();
    result.addAll(events);
    if (events.length < 500) return result;
    final next = events.last;
    if (cursor != null &&
        (next.createdAt > cursor.createdAt ||
            (next.createdAt == cursor.createdAt &&
                next.id.compareTo(cursor.id) <= 0))) {
      throw StateError('Membership pagination did not advance');
    }
    cursor = next;
  }
  throw StateError('Membership query exceeded its page budget');
}
