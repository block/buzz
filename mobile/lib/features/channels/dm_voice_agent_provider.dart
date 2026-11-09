import 'dart:async';

import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/me/me_agent_inventory.dart';
import '../../shared/me/me_assistants.dart';
import '../../shared/me/me_sync.dart';
import '../../shared/relay/relay.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/profile/user_cache_provider.dart';
import 'channel.dart';
import 'channel_management_provider.dart';
import 'channels_provider.dart';

/// Runtime is owner-signed discovery evidence, not permission to run an agent.
/// The hosting desktop rechecks its live runtime, protection and room access.
final ownedCodexVoiceAgentsProvider = FutureProvider<Set<String>>((ref) async {
  final sync = ref.watch(meSyncProvider);
  ref.watch(_voiceInventoryChangesProvider);
  ref.watch(relaySessionProvider.select((state) => state.status));
  final inventory = await loadMeAgentInventory(
    sync,
    runtime: 'buzz.codex/codex',
  );
  final owned = await ref.refresh(meAssistantsProvider.future);
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

// Subscribe separately from the snapshot so refreshing it does not replay the
// subscription and cause an invalidation loop. Reconnects resubscribe.
final _voiceInventoryChangesProvider = StreamProvider.autoDispose<int>((ref) {
  final sync = ref.watch(meSyncProvider);
  ref.watch(relaySessionProvider.select((state) => state.status));
  final updates = StreamController<int>();
  var revision = 0;
  var disposed = false;
  void Function()? unsubscribe;
  unawaited(() async {
    try {
      unsubscribe = await sync.session.subscribe(
        NostrFilter(kinds: const [30177], authors: [sync.pubkey], limit: 0),
        (_) {
          if (!disposed) updates.add(++revision);
        },
      );
      if (disposed) unsubscribe?.call();
    } catch (error, stack) {
      if (!disposed) updates.addError(error, stack);
    }
  }());
  ref.onDispose(() {
    disposed = true;
    unsubscribe?.call();
    unawaited(updates.close());
  });
  return updates.stream;
});

/// Classification only: ordinary human/group DMs never depend on inventory
/// network availability. Admission still requires fresh signed evidence below.
final dmVoiceCandidateProvider = Provider.autoDispose.family<String?, String>((
  ref,
  channelId,
) {
  final channel = ref.watch(channelByIdProvider(channelId));
  if (channel == null) return null;
  final known = ref.watch(agentMentionPubkeysProvider(channelId));
  final profiles = ref.watch(userCacheProvider);
  final owned =
      ref.watch(ownedCodexVoiceAgentsProvider).value ?? const <String>{};
  return dmVoiceAgent(channel, ref.watch(currentPubkeyProvider), {
    ...known,
    ...owned,
    for (final profile in profiles.values)
      if (profile.ownerPubkey != null) profile.pubkey,
  });
});

/// All new-call paths, including recovery, resolve fresh signed evidence.
Future<String?> resolveDmVoiceAgent(Ref ref, String parentChannelId) async {
  final channel = ref.read(channelByIdProvider(parentChannelId));
  if (channel == null ||
      ref.read(dmVoiceCandidateProvider(parentChannelId)) == null) {
    return null;
  }
  final agents = await ref.refresh(ownedCodexVoiceAgentsProvider.future);
  final agent = dmVoiceAgent(channel, ref.read(currentPubkeyProvider), agents);
  if (agent == null) {
    throw StateError('This agent is no longer available for voice.');
  }
  return agent;
}
