import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/agents/remote_agents.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list_card.dart';
import 'agent_instructions.dart';

/// The agent's instructions and management information in shared Settings cards.
class AgentIdentitySections extends HookWidget {
  const AgentIdentitySections({
    required this.pubkey,
    required this.controller,
    super.key,
  });

  final String pubkey;
  final RemoteAgentController controller;

  @override
  Widget build(BuildContext context) {
    useListenable(controller);
    final remote = controller.agents.any(
      (agent) => agent['agent_pubkey'] == pubkey,
    );
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
                child: AgentInstructions(pubkey: pubkey),
              ),
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
                  remote
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
