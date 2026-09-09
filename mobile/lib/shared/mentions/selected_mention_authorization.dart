part of 'agent_identity_provider.dart';

/// Evidence requirements, not a claim that an ordinary identity is human.
enum SelectedMentionKind { ordinary, agent, unresolvedAgent }

/// Fresh evidence for one exact selected key. This is NOT permission to write:
/// consumers must apply their existing agent eligibility evaluator to [agent],
/// normal channel invitation permission, consent and scope fences separately.
/// An existing member needs no role write, whatever their current role is.
class SelectedMentionAuthorization {
  final SelectedMentionKind kind;
  final bool isMember;

  /// Policy projection with relay-authenticated destination membership only.
  /// Null for ordinary identities or unresolved prior-agent evidence. Missing
  /// owner policy produces a deny-all entry, never runtime fallback.
  final AgentDirectoryEntry? agent;

  const SelectedMentionAuthorization._(this.kind, this.isMember, this.agent);

  bool get requiresAgentAuthorization => kind != SelectedMentionKind.ordinary;

  /// Role selection only, never an invitation authorization. A consumer must
  /// first validate eligibility and consent and skip writes for existing members.
  String? get invitationRole => switch (kind) {
    SelectedMentionKind.ordinary => 'member',
    SelectedMentionKind.agent => 'bot',
    SelectedMentionKind.unresolvedAgent => null,
  };
}

/// Read all selected keys, not just candidates whose saved isAgent bit is true.
/// [priorAgentKeys] is denial-only taint: lost provenance cannot turn a saved
/// agent capability into an ordinary-member invitation or notification bypass.
/// Absence of agent evidence permits the existing ordinary membership flow,
/// NOT a humanity assertion. Positive fresh evidence overrides saved false.
///
/// Every requested key has a result or the entire read throws. This read makes
/// no cache writes and does not opt into resolveProfileOwners. Check [isCurrent]
/// again before each side effect; re-read after consent and accepted invitations.
Future<Map<String, SelectedMentionAuthorization>>
readSelectedMentionAuthorization(
  RelaySessionNotifier session,
  Set<String> requestedKeys, {
  required String viewer,
  required String channelId,
  Set<String> priorAgentKeys = const {},
  required bool Function() isCurrent,
}) async {
  void check() {
    if (!isCurrent()) throw StateError('Mention authorization scope changed');
  }

  check();
  if (requestedKeys.length > 1000 ||
      !requestedKeys.containsAll(priorAgentKeys) ||
      requestedKeys.any((key) => !RegExp(r'^[0-9a-f]{64}$').hasMatch(key)) ||
      !RegExp(r'^[0-9a-f]{64}$').hasMatch(viewer) ||
      channelId.isEmpty) {
    throw StateError('Invalid selected mention request');
  }
  if (requestedKeys.isEmpty) return const {};
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
  NostrEvent? roster;
  for (final event in membership) {
    if (event.kind != 39002 ||
        event.pubkey != authority ||
        event.getTagValue('d') != channelId) {
      continue;
    }
    if (roster == null || _newer(event, roster)) roster = event;
  }
  // A malformed newest head is unavailable, never permission to invite someone
  // whose current membership could not be established. Missing head is likewise
  // not a trustworthy empty roster at this destination.
  if (roster == null ||
      !verifySignedEvent(roster) ||
      roster.tags.where((t) => t.isNotEmpty && t[0] == 'd').length != 1) {
    throw StateError('Destination membership unavailable');
  }
  final members = <String, String>{};
  for (final tag in roster.tags.where((t) => t.isNotEmpty && t[0] == 'p')) {
    if (tag.length < 2 || members.containsKey(tag[1])) {
      throw StateError('Ambiguous destination membership');
    }
    members[tag[1]] = tag.length >= 4 ? tag[3] : 'member';
  }
  final runtime = await _queryAgentFilters(session, [
    for (final key in requestedKeys)
      NostrFilter(kinds: const [10100], authors: [key], limit: 1),
  ], checkCurrent: check);
  check();
  Map<String, NostrEvent> profiles = const {};
  final policies = await resolveAgentPolicies(
    session,
    runtime.where((e) => requestedKeys.contains(e.pubkey)).toList(),
    requestedKeys: requestedKeys,
    checkCurrent: check,
    onProfileEvidence: (value) => profiles = value,
  );
  check();
  final latestRuntime = <String, NostrEvent>{};
  for (final event in runtime) {
    if (event.kind != 10100 || !requestedKeys.contains(event.pubkey)) continue;
    final previous = latestRuntime[event.pubkey];
    if (previous == null || _newer(event, previous)) {
      latestRuntime[event.pubkey] = event;
    }
  }
  final byKey = {for (final policy in policies) policy.pubkey: policy};
  final result = <String, SelectedMentionAuthorization>{};
  for (final key in requestedKeys) {
    final profile = profiles[key];
    final owner = profile == null ? null : verifiedOaOwnerPubkey(profile);
    final invalidProfile =
        profile != null &&
        (!verifySignedEvent(profile) ||
            (owner == null &&
                profile.tags.any((t) => t.isNotEmpty && t[0] == 'auth')));
    final runtimeHead = latestRuntime[key];
    final knownAgent =
        owner != null ||
        members[key] == 'bot' ||
        (runtimeHead != null && verifySignedEvent(runtimeHead));
    final unresolved =
        invalidProfile ||
        (runtimeHead != null && !verifySignedEvent(runtimeHead)) ||
        (!knownAgent && priorAgentKeys.contains(key));
    final kind = unresolved
        ? SelectedMentionKind.unresolvedAgent
        : knownAgent
        ? SelectedMentionKind.agent
        : SelectedMentionKind.ordinary;
    AgentDirectoryEntry? agent;
    if (kind == SelectedMentionKind.agent) {
      final policy = byKey[key];
      // Ownership without a matching valid policy must not revive headless
      // runtime authority. mergeAgentPolicies remains the sole policy parser.
      final missingOwnedPolicy = owner != null && policy?.ownerPubkey != owner;
      agent = AgentDirectoryEntry(
        pubkey: key,
        ownerPubkey: owner,
        displayName: policy?.displayName,
        respondTo: missingOwnedPolicy ? 'nobody' : policy?.respondTo,
        respondToAllowlist: missingOwnedPolicy
            ? const []
            : policy?.respondToAllowlist ?? const [],
        channelIds:
            members.containsKey(viewer) &&
                members.containsKey(key) &&
                (owner == viewer || members[key] == 'bot')
            ? [channelId]
            : const [],
      );
    }
    result[key] = SelectedMentionAuthorization._(
      kind,
      members.containsKey(key),
      agent,
    );
  }
  check();
  return Map.unmodifiable(result);
}
