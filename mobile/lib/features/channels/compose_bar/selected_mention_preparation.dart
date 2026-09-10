part of '../compose_bar.dart';

// One preparation owner retains the exact evidence and denial-only taint across
// consent, individual invitations and final publication reads.
Future<Map<String, SelectedMentionAuthorization>> _authorizeSelectedMentions(
  Set<String> keys, {
  required SelectedMentionAuthorizationReader readSelected,
  required Set<String> priorAgentKeys,
  required Set<String> observedKeys,
  required Map<String, NostrEvent> observedProfiles,
  required String? currentPubkey,
  required String channelId,
  required VoidCallback ensureAuthorizationCurrent,
  required bool Function() isAuthorizationCurrent,
  bool prepare = false,
}) async {
  ensureAuthorizationCurrent();
  if (keys.isEmpty) return const {};
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
    ensureAuthorizationCurrent();
    if (evidence.length != keys.length ||
        !evidence.keys.toSet().containsAll(keys)) {
      throw Exception('Incomplete selected mention evidence');
    }
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
    ensureAuthorizationCurrent();
    rethrow;
  }
}
