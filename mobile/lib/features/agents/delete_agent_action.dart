import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/agents/remote_agents.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';

/// Confirmed deletion for the signed-in owner's remote agents only.
class DeleteAgentAction extends HookConsumerWidget {
  const DeleteAgentAction({
    required this.controller,
    required this.pubkey,
    super.key,
  });

  final RemoteAgentController controller;
  final String pubkey;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useListenable(controller);
    final error = useState<String?>(null);
    final community = ref.watch(activeCommunityProvider).asData?.value;
    final agent = controller.agents
        .where((row) => row['agent_pubkey'] == pubkey)
        .firstOrNull;
    if (community == null ||
        community.pubkey == null ||
        community.pubkey != controller.owner ||
        agent == null ||
        controller.account.account == null) {
      return const SizedBox.shrink();
    }
    Future<void> remove() async {
      final confirmed = await showDialog<bool>(
        context: context,
        builder: (dialogContext) => AlertDialog(
          title: Text('Delete ${agent['agent_name'] ?? 'agent'}?'),
          content: const Text(
            'Permanently delete this remote agent and its registration in this community? Message history and memberships remain. Registrations in other communities are not removed.',
          ),
          actions: [
            TextButton(
              onPressed: () => Navigator.pop(dialogContext, false),
              child: const Text('Cancel'),
            ),
            TextButton(
              onPressed: () => Navigator.pop(dialogContext, true),
              style: TextButton.styleFrom(
                foregroundColor: context.colors.error,
              ),
              child: const Text('Delete'),
            ),
          ],
        ),
      );
      if (confirmed != true || !context.mounted) return;
      error.value = null;
      final success = await controller.deleteAgent(
        pubkey: pubkey,
        community: community,
        isCurrent: () =>
            context.mounted &&
            ref.read(activeCommunityProvider).asData?.value?.id == community.id,
        fetch: ref.read(relaySessionProvider.notifier).fetchHistory,
      );
      if (!context.mounted) return;
      ref.invalidate(agentDirectoryProvider);
      if (success) {
        Navigator.of(context).pop();
      } else {
        error.value = controller.status;
      }
    }

    return Padding(
      padding: EdgeInsets.zero,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          FilledButton(
            onPressed: controller.busy ? null : remove,
            style: FilledButton.styleFrom(
              minimumSize: const Size(double.infinity, Grid.xxl),
              padding: const EdgeInsets.symmetric(
                horizontal: Grid.gutter,
                vertical: Grid.xs,
              ),
              backgroundColor: context.colors.surfaceContainerHighest,
              foregroundColor: context.colors.error,
              textStyle: context.textTheme.titleSmall,
              shape: RoundedRectangleBorder(
                borderRadius: BorderRadius.circular(Radii.dialog),
              ),
            ),
            child: Text(controller.busy ? 'Working…' : 'Delete'),
          ),
          if (error.value != null)
            Text(error.value!, style: TextStyle(color: context.colors.error)),
        ],
      ),
    );
  }
}
