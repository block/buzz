part of 'agent_identity_provider.dart';

bool _newer(NostrEvent event, NostrEvent previous) =>
    event.createdAt > previous.createdAt ||
    (event.createdAt == previous.createdAt &&
        event.id.compareTo(previous.id) < 0);

/// Exact-coordinate queries, bounded to ten filters in flight. An empty result
/// is distinct from a failed read; failures must never revive runtime access.
Future<List<NostrEvent>> _queryAgentFilters(
  RelaySessionNotifier session,
  List<NostrFilter> filters, {
  void Function()? checkCurrent,
}) async {
  final events = <NostrEvent>[];
  for (var start = 0; start < filters.length; start += 10) {
    checkCurrent?.call();
    events.addAll(
      await session.queryRelay(filters.skip(start).take(10).toList()),
    );
  }
  return events;
}

/// Overlay current owner-authenticated policy onto the existing runtime
/// directory. This does not expand discovery to owner-only coordinates yet.
Future<List<AgentDirectoryEntry>> resolveAgentPolicies(
  RelaySessionNotifier session,
  List<NostrEvent> runtimeEvents, {
  Set<String>? requestedKeys,
  void Function()? checkCurrent,
}) async {
  final latest = <String, NostrEvent>{};
  for (final event in runtimeEvents.where((event) => event.kind == 10100)) {
    final previous = latest[event.pubkey];
    if (previous == null || _newer(event, previous)) {
      latest[event.pubkey] = event;
    }
  }
  final keys = requestedKeys ?? latest.keys.toSet();
  final profiles = latestProfileEvents(
    await _queryAgentFilters(session, [
      for (final key in keys)
        NostrFilter(kinds: const [0], authors: [key], limit: 1),
    ], checkCurrent: checkCurrent),
  );
  final owners = <String, String>{};
  for (final profile in profiles.values) {
    final owner = verifiedOaOwnerPubkey(profile);
    if (owner != null && keys.contains(profile.pubkey)) {
      owners[profile.pubkey] = owner;
    }
  }
  checkCurrent?.call();
  final policies = await _queryAgentFilters(session, [
    for (final owner in owners.entries)
      NostrFilter(
        kinds: const [30177],
        authors: [owner.value],
        tags: {
          '#d': [owner.key],
        },
        limit: 1,
      ),
  ], checkCurrent: checkCurrent);
  checkCurrent?.call();
  return mergeAgentPolicies(latest.values, policies, owners);
}

/// Latest signed owner policy is the access authority, not runtime claims.
/// Malformed/revoked policy reserves the identity with deny-all permissions;
/// it must not fall back to an older permissive policy or kind:10100 record.
List<AgentDirectoryEntry> mergeAgentPolicies(
  Iterable<NostrEvent> runtimeEvents,
  Iterable<NostrEvent> policies,
  Map<String, String> verifiedOwners,
) {
  final latestPolicies = <String, NostrEvent>{};
  for (final event in policies) {
    final key = event.getTagValue('d');
    if (key == null || verifiedOwners[key] != event.pubkey) continue;
    final previous = latestPolicies[key];
    if (previous == null || _newer(event, previous)) {
      latestPolicies[key] = event;
    }
  }
  final agents = <String, AgentDirectoryEntry>{};
  for (final event in runtimeEvents) {
    if (event.kind != 10100 || !verifySignedEvent(event)) continue;
    final data = _tryDecodeJsonMap(event.content);
    if (data == null) continue;
    agents[event.pubkey] = AgentDirectoryEntry(
      pubkey: event.pubkey,
      displayName: data['display_name'] is String
          ? data['display_name'] as String
          : data['name'] is String
          ? data['name'] as String
          : null,
      respondTo: data['respond_to'] is String
          ? data['respond_to'] as String
          : null,
      respondToAllowlist: _stringList(data['respond_to_allowlist']) ?? const [],
      channelIds: _stringList(data['channel_ids']) ?? const [],
    );
  }
  for (final entry in latestPolicies.entries) {
    final event = entry.value;
    final data = _tryDecodeJsonMap(event.content);
    final mode = data?['respond_to'];
    final parallelism = data?['parallelism'];
    final allowlist = _stringList(data?['respond_to_allowlist'] ?? []);
    final valid =
        event.kind == 30177 &&
        verifySignedEvent(event) &&
        event.tags.where((tag) => tag.isNotEmpty && tag[0] == 'd').length ==
            1 &&
        data?['name'] is String &&
        parallelism is int &&
        parallelism >= 0 &&
        parallelism <= 4294967295 &&
        const ['owner-only', 'allowlist', 'anyone', 'nobody'].contains(mode) &&
        allowlist != null &&
        const [
          'persona_id',
          'system_prompt',
          'model',
          'provider',
          'persona_source_version',
        ].every((key) => data?[key] == null || data?[key] is String);
    agents[entry.key] = AgentDirectoryEntry(
      pubkey: entry.key,
      ownerPubkey: event.pubkey,
      displayName: valid
          ? data!['name'] as String
          : agents[entry.key]?.displayName,
      respondTo: valid ? mode as String : 'nobody',
      respondToAllowlist: valid ? allowlist : const [],
      channelIds: agents[entry.key]?.channelIds ?? const [],
    );
  }
  return agents.values.toList();
}

List<String>? _stringList(Object? value) =>
    value is List && value.every((v) => v is String)
    ? value.cast<String>().map((v) => v.toLowerCase()).toList()
    : null;
