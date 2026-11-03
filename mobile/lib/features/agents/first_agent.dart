import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../shared/agents/owned_agent_directory.dart';
import '../../shared/agents/remote_agents.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/community/community.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/hosted/hosted_environment.dart';
import '../channels/channel.dart';
import '../channels/channel_management_provider.dart';
import '../channels/send_message_provider.dart';

/// What a first agent is told to do. Edit freely; it is sent once, at setup.
const starterAgentInstructions = '''
You are the person's first agent in Buzz. When they say hello, greet them by
your name and briefly explain Buzz: channels are shared spaces for a team,
DMs are private conversations, and agents like you can be messaged or
mentioned anywhere to help. Tell them they can ask you anything about how
Buzz works, and point them to the community's starter channels to look
around. Keep replies short, friendly and plain text only: no Markdown,
tables or code blocks.''';

/// The first message, sent as the person, so the new agent wakes and greets.
const starterAgentHello = 'Hi! What can you help me with in Buzz?';

/// Builderlab has no default model, so the first agent uses the catalog's
/// first model, at medium effort when it supports it.
({String model, String effort})? starterModel(List<RemoteAgentModel> catalog) {
  final model = catalog.firstOrNull;
  if (model == null) return null;
  return (
    model: model.id,
    effort: model.efforts.contains('medium') ? 'medium' : model.efforts.first,
  );
}

/// First-agent setup, kept outside any page so leaving the setup view never
/// loses the picks or interrupts the setup.
@immutable
class FirstAgentState {
  const FirstAgentState({
    this.name = '',
    this.picture,
    this.working = false,
    this.error,
    this.community,
    this.agent,
    this.ready,
  });

  /// Picks made so far, restored when the setup view reopens.
  final String name;
  final String? picture;
  final bool working;
  final String? error;

  /// The community the setup started in, and the agent it created there.
  final String? community;
  final String? agent;

  /// The new agent's DM, once it has been opened and greeted.
  final Channel? ready;

  FirstAgentState copyWith({
    bool? working,
    String? error,
    String? community,
    String? agent,
    Channel? ready,
  }) => FirstAgentState(
    name: name,
    picture: picture,
    working: working ?? false,
    error: error,
    community: community ?? this.community,
    agent: agent ?? this.agent,
    ready: ready ?? this.ready,
  );
}

class FirstAgentSetup extends Notifier<FirstAgentState> {
  @override
  FirstAgentState build() => const FirstAgentState();

  void pick({required String name, String? picture}) {
    if (state.working || state.agent != null) return;
    state = FirstAgentState(name: name, picture: picture, error: state.error);
  }

  /// Creates the agent, opens its DM and says hello as the person, so the
  /// agent wakes and greets them. Once the agent exists, retrying only
  /// reopens its DM, so a second agent is never created.
  Future<void> start(Community community) async {
    if (state.working || state.ready != null) return;
    if (state.community != null && state.community != community.id) return;
    state = state.copyWith(working: true, community: community.id);
    final controller = ref.read(remoteAgentControllerProvider);
    var agent = state.agent;
    if (agent == null) {
      final choice = starterModel(await _catalog(controller));
      final created =
          choice != null &&
          await controller.create(
            community,
            state.name,
            instructions: starterAgentInstructions,
            pictureUrl: state.picture,
            modelId: choice.model,
            reasoningEffort: choice.effort,
          );
      if (!created) {
        final failure = choice == null ? '' : controller.failure;
        state = state.copyWith(
          error: failure.isEmpty
              ? 'Could not start setting up your agent. Try again.'
              : failure,
        );
        return;
      }
      agent = controller.createdAgent!;
      state = state.copyWith(working: true, agent: agent);
      ref.invalidate(agentDirectoryProvider);
      ref.invalidate(ownedAgentDirectoryProvider);
    }
    try {
      final channel = await ref
          .read(channelActionsProvider)
          .openDm(pubkeys: [agent]);
      await ref
          .read(sendMessageProvider)
          .call(
            channelId: channel.id,
            content: starterAgentHello,
            mentionPubkeys: [agent],
            channel: channel,
          );
      state = state.copyWith(ready: channel);
    } catch (_) {
      state = state.copyWith(
        error:
            'Your agent is ready, but its conversation did not open. '
            'Try again.',
      );
    }
  }

  Future<List<RemoteAgentModel>> _catalog(
    RemoteAgentController controller,
  ) async {
    try {
      return await controller.modelCatalog();
    } catch (_) {
      return const [];
    }
  }
}

final firstAgentSetupProvider =
    NotifierProvider<FirstAgentSetup, FirstAgentState>(FirstAgentSetup.new);

/// Whether DMs offers "Create your first agent": only once the registry has
/// loaded and shows this account owns no remote agent, in a supported
/// community the account owns, or while a setup started here is unfinished.
final showFirstAgentProvider = Provider<bool>((ref) {
  if (!ref.watch(remoteAgentsEnabledProvider)) return false;
  final community = ref.watch(activeCommunityProvider).asData?.value;
  final controller = ref.watch(remoteAgentControllerProvider);
  final setup = ref.watch(firstAgentSetupProvider);
  void changed() => ref.invalidateSelf();
  controller.addListener(changed);
  ref.onDispose(() => controller.removeListener(changed));
  if (community == null) return false;
  if (setup.community == community.id && setup.ready == null) {
    if (setup.working || setup.error != null) return true;
  }
  // Load only while the listing is unknown, so a failure hides the row
  // instead of retrying in a loop. Loading notifies, so never during build.
  if (!controller.listingLoaded && !controller.listingFailed) {
    scheduleMicrotask(() => controller.load(community.relayUrl));
  }
  return controller.listingLoaded &&
      !controller.listingFailed &&
      controller.owner != null &&
      controller.owner == community.pubkey &&
      controller.activeAgents.isEmpty;
});

/// Records that the setup view opened by itself for this account, so it does
/// so only once.
Future<bool> claimFirstAgentAutoOpen(RemoteAgentController controller) async {
  final subject = controller.account.account?.subject;
  if (subject == null) return false;
  final key = 'first-agent-offered:${HostedEnvironment.origin.host}:$subject';
  final prefs = await SharedPreferences.getInstance();
  if (prefs.getBool(key) == true) return false;
  return prefs.setBool(key, true);
}
