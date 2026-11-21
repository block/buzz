import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/agents/remote_agents.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/theme/buzz_icons.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';
import '../../shared/widgets/text_editor.dart';
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
    final unknown = agentLocationUnknown(community, controller);
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
    final editable = row != null &&
        community != null &&
        supportsRemoteAgents(community) &&
        controller.owner == community.pubkey &&
        controller.activeAgents.contains(pubkey);
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
            if (editable)
              AppListRow(
                key: const ValueKey('edit-agent-instructions-row'),
                title: 'Edit instructions',
                trailing: const Icon(BuzzIcons.chevronRight, size: 18),
                onTap: () async {
                  await showTextEditor(
                    context: context,
                    title: 'Instructions',
                    initialValue: row['agent_instructions'] as String? ?? '',
                    hintText: 'Describe what this agent should do.',
                    multiline: true,
                    keyPrefix: 'agent-instructions',
                    onSave: (value) async {
                      final ok = await controller.updateInstructions(
                        community: community,
                        pubkey: pubkey,
                        instructions: value,
                        isCurrent: () =>
                            context.mounted &&
                            community.pubkey == controller.owner,
                      );
                      if (!ok) {
                        throw StateError(
                          controller.failure.isNotEmpty
                              ? controller.failure
                              : 'Could not update instructions',
                        );
                      }
                    },
                  );
                },
              ),
          ],
        ),
        AppListCard(
          label: 'Management',
          horizontalPadding: 0,
          children: [
            Padding(
              padding: const EdgeInsets.all(Grid.xs),
              child: SizedBox(
                width: double.infinity,
                child: Text(
                  unknown && !remote && controller.busy
                      ? 'Checking whether this agent runs on Builderbot…'
                      : unknown && !remote
                      ? 'Sign in to Builderlab to see whether this agent runs on Builderbot or your desktop.'
                      : remote
                      ? 'This agent runs on Builderbot. Your desktop doesn’t need to be online for it to work.'
                      : 'This agent is managed on your desktop. Keep Buzz open and your desktop online for it to work.',
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
