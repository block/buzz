part of '../compose_bar.dart';

// One preparation owner retains the exact evidence and denial-only taint across
// consent, individual invitations and final publication reads.
Future<Map<String, SelectedMentionAuthorization>> _authorizeSelectedMentions(
  Set<String> keys, {
  required SelectedMentionAuthorizationReader readSelected,
  required Set<String> priorAgentKeys,
  required Set<String> observedKeys,
  required Map<String, NostrEvent> observedProfiles,
  required Set<bool Function()> evidenceChecks,
  required String? currentPubkey,
  required String channelId,
  required VoidCallback ensureAuthorizationCurrent,
  required VoidCallback ensureScopeCurrent,
  required bool Function() isAuthorizationCurrent,
  bool prepare = false,
}) async {
  ensureScopeCurrent();
  if (keys.isEmpty) {
    evidenceChecks.clear();
    return const {};
  }
  try {
    final evidence = await readSelected(
      keys,
      priorAgentKeys.intersection(keys),
      currentPubkey ?? '',
      channelId,
      isAuthorizationCurrent,
      (profiles) {
        for (final key in keys) {
          if (observedKeys.contains(key) &&
              observedProfiles[key]?.id != profiles[key]?.id) {
            throw Exception('Mention evidence changed; retry the draft');
          }
        }
        observedKeys.addAll(keys);
        observedProfiles.addAll(profiles);
      },
    );
    ensureScopeCurrent();
    if (evidence.length != keys.length ||
        !evidence.keys.toSet().containsAll(keys)) {
      throw Exception('Incomplete selected mention evidence');
    }
    evidenceChecks
      ..clear()
      ..addAll(evidence.values.map((value) => value.isCurrent));
    ensureAuthorizationCurrent();
    final agents = {
      for (final key in keys)
        if (evidence[key]!.requiresAgentAuthorization) key,
    };
    priorAgentKeys.addAll(agents);
    await authorizeAgentMentions(
      (_, _, _, _) async => [for (final key in agents) ?evidence[key]!.agent],
      agents,
      currentPubkey,
      channelId,
      isAuthorizationCurrent,
      prepare: prepare,
    );
    ensureAuthorizationCurrent();
    return evidence;
  } catch (_) {
    ensureScopeCurrent();
    rethrow;
  }
}

// Failures preserve the exact draft audience; consent never licenses a changed
// role. Kept beside the reader owner so both boundaries use one evaluator.
Future<void> _prepareMentionInvitations({
  required _OutgoingMentions outgoing,
  required _NonMemberMentionScan scan,
  required ChannelActions channelActions,
  required ScaffoldMessengerState? messenger,
  required Future<Map<String, SelectedMentionAuthorization>> Function(
    Set<String> keys, {
    bool prepare,
  })
  authorize,
  required VoidCallback ensureCurrent,
  required VoidCallback ensureScopeCurrent,
  required VoidCallback onStarted,
  required bool fenceAfterPreparation,
}) async {
  final keys = outgoing.pubkeys.toSet();
  Future<bool> authorizeWrite(String key, String role) async {
    final evidence = await authorize(keys, prepare: true);
    final fresh = evidence[key]!;
    if (fresh.invitationRole != role) {
      throw Exception(
        'Mention classification changed; retry invitation consent',
      );
    }
    return !fresh.isMember;
  }

  await authorize(keys, prepare: true);
  ensureCurrent();
  onStarted();
  await outgoing.addNonMembers(
    channelActions,
    scan: scan,
    messenger: messenger,
    ensureCurrent: ensureCurrent,
    ensureScopeCurrent: ensureScopeCurrent,
    authorizeWrite: authorizeWrite,
  );
  if (!outgoing.pubkeys.toSet().containsAll(keys)) {
    throw Exception(
      'Mention invitation failed. Draft kept; retry or remove the mention.',
    );
  }
  await authorize(keys);
  if (fenceAfterPreparation) ensureCurrent();
}

// Equivalent refreshes retain scope; destination/credentials do not.
bool _isComposeConfigCurrent(WidgetRef ref, RelayConfig config) {
  final current = ref.read(relayConfigProvider);
  return current.baseUrl == config.baseUrl && current.nsec == config.nsec;
}
