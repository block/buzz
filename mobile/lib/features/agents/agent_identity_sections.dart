import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/agents/remote_agents.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';
import 'agent_instructions.dart';

/// The agent's instructions and management information in shared Settings cards.
class AgentIdentitySections extends HookConsumerWidget {
  const AgentIdentitySections({
    required this.pubkey,
    required this.controller,
    super.key,
  });

  final String pubkey;
  final RemoteAgentController controller;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(controller);
    useListenable(controller.account.subjectChanges);
    final community = ref.watch(activeCommunityProvider).asData?.value;
    final signedOut = agentLocationUnknown(community, controller);
    final unconfirmed = agentLocationUnconfirmed(community, controller);
    final remote = controller.agents.any(
      (agent) => agent['agent_pubkey'] == pubkey,
    );
    // Remote instructions come from the registry row, listed for the active
    // community's owner only (as the agents page decides).
    final owner = community?.pubkey;
    final row = owner != null && controller.owner == owner
        ? controller.agents
              .where((agent) => agent['agent_pubkey'] == pubkey)
              .firstOrNull
        : null;
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: [
        AppListCard(
          label: 'Instructions',
          horizontalPadding: 0,
          children: [
            Padding(
              padding: const EdgeInsets.all(Grid.xs),
              child: SizedBox(
                width: double.infinity,
                child: AgentInstructions(pubkey: pubkey, registryRow: row),
              ),
            ),
          ],
        ),
        AppListCard(
          label: 'Management',
          horizontalPadding: 0,
          children: [
            if (remote)
              // Desktop's remote settings card: where it runs, then what that
              // means, since mobile has no composer popover to explain it.
              const AppListRow(
                icon: BuzzIcons.cloud,
                title: 'Remote agent',
                subtitle:
                    'Runs on a Builderlab machine, so you can chat with it from here anytime.',
              )
            else
              Padding(
                padding: const EdgeInsets.all(Grid.xs),
                child: SizedBox(
                  width: double.infinity,
                  child: Text(
                    unconfirmed && controller.busy
                        ? 'Checking whether this agent runs on Builderlab…'
                        : signedOut
                        ? 'Sign in to Builderlab to see whether this agent runs remotely or on another of your devices.'
                        : unconfirmed
                        ? 'Couldn’t check whether this agent runs on Builderlab. Pull down on Agents to try again.'
                        : 'Runs on another of your devices, so you can chat with it from here while Buzz is open there. Change or delete it on that device.',
                    style: context.textTheme.bodyMedium,
                  ),
                ),
              ),
          ],
        ),
      ],
    );
  }
}
