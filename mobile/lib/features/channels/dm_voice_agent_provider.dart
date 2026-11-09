import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/me/me_agent_inventory.dart';
import '../../shared/me/me_assistants.dart';
import '../../shared/me/me_sync.dart';
import '../../shared/relay/relay.dart';
import 'channel.dart';

/// Runtime is owner-signed discovery evidence, not permission to run an agent.
/// The hosting desktop rechecks its live runtime, protection and room access.
final ownedCodexVoiceAgentsProvider = FutureProvider<Set<String>>((ref) async {
  final sync = ref.watch(meSyncProvider);
  ref.watch(relaySessionProvider.select((state) => state.status));
  final inventory = await loadMeAgentInventory(
    sync,
    runtime: 'buzz.codex/codex',
  );
  final owned = await ref.watch(meAssistantsProvider.future);
  sync.check();
  return {
    for (final agent in owned)
      if (inventory.containsKey(agent.pubkey)) agent.pubkey,
  };
});

/// Only the viewer's Codex peer in a writable one-to-one DM is offered voice.
String? dmVoiceAgent(Channel channel, String? viewer, Set<String> ownedCodex) {
  final members = channel.participantPubkeys.toSet();
  if (!channel.isDm ||
      channel.isArchived ||
      !channel.isMember ||
      members.length != 2 ||
      !members.contains(viewer)) {
    return null;
  }
  final peer = members.firstWhere((key) => key != viewer);
  return ownedCodex.contains(peer) ? peer : null;
}
