part of 'agent_identity_provider.dart';

/// Fresh reader used by the composer; suggestions never authorize publication.
typedef AgentAuthorizationReader =
    Future<List<AgentDirectoryEntry>> Function(
      Set<String> keys,
      String? viewer,
      String channelId,
      bool Function() isCurrent,
    );

/// Session-bound fresh authorization reader for exact recipient keys and one
/// destination. Re-reads verified ownership, policy and relay-signed membership;
/// cached directory suggestions are never publication authority. Query and
/// scope-change failures propagate to the caller, which must check currentness
/// before applying membership or publishing. This is not an atomic relay write.
final agentAuthorizationReaderProvider = Provider<AgentAuthorizationReader>((
  ref,
) {
  final session = ref.watch(relaySessionProvider.notifier);
  return (keys, viewer, channelId, isCurrent) => readAgentAuthorization(
    session,
    keys,
    viewer: viewer,
    channelId: channelId,
    isCurrent: isCurrent,
  );
});

/// Verify every intended recipient; never silently shrink the notification set.
Future<void> authorizeAgentMentions(
  AgentAuthorizationReader read,
  Set<String> keys,
  String? viewer,
  String channelId,
  bool Function() isCurrent, {
  bool prepare = false,
}) async {
  if (keys.isEmpty) return;
  const message =
      'Could not authorize a mentioned agent. Check its access and channel membership, then retry or remove the mention.';
  try {
    if (!isCurrent()) throw Exception(message);
    final agents = await read(keys, viewer, channelId, isCurrent);
    if (!isCurrent()) throw Exception(message);
    for (final key in keys) {
      final agent = agents.where((a) => a.pubkey == key).firstOrNull;
      final owned = agent?.ownerPubkey != null && agent?.ownerPubkey == viewer;
      final allowed =
          agent != null &&
          ((owned &&
                  const [
                    'owner-only',
                    'allowlist',
                    'anyone',
                  ].contains(agent.respondTo)) ||
              (agent.respondTo == 'allowlist' &&
                  agent.respondToAllowlist.contains(viewer)) ||
              (agent.respondTo == 'anyone' &&
                  agent.channelIds.contains(channelId)));
      if (!allowed ||
          (!(prepare && owned) && !agent.channelIds.contains(channelId))) {
        throw Exception(message);
      }
    }
  } catch (_) {
    throw Exception(message);
  }
}

/// Session-bound all-key evidence reader. Saved agent keys are denial-only taint.
typedef SelectedMentionAuthorizationReader =
    Future<Map<String, SelectedMentionAuthorization>> Function(
      Set<String> keys,
      Set<String> priorAgentKeys,
      String viewer,
      String channelId,
      bool Function() isCurrent,
      void Function(Map<String, NostrEvent>) onProfileEvidence,
    );

/// Read-only publication seam; does not populate suggestion/profile caches.
final selectedMentionAuthorizationReaderProvider =
    Provider<SelectedMentionAuthorizationReader>((ref) {
      final session = ref.watch(relaySessionProvider.notifier);
      return (keys, prior, viewer, channel, current, observed) =>
          readSelectedMentionAuthorization(
            session,
            keys,
            viewer: viewer,
            channelId: channel,
            priorAgentKeys: prior,
            isCurrent: current,
            onProfileEvidence: observed,
          );
    });
