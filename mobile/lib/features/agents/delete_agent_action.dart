import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/agents/remote_agents.dart';
import '../../shared/community/community_provider.dart';
import '../../shared/mentions/agent_identity_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import '../../shared/widgets/buzz_navigation_metrics.dart';
import '../../shared/theme/buzz_icons.dart';

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
        await showDialog<void>(
          context: context,
          builder: (dialogContext) => AlertDialog(
            title: const Text('Could not delete agent'),
            content: Text(controller.failure),
            actions: [
              TextButton(
                onPressed: () => Navigator.pop(dialogContext),
                child: const Text('OK'),
              ),
            ],
          ),
        );
      }
    }

    if (Theme.of(context).platform == TargetPlatform.iOS) {
      return IosGlassNavigationButton(
        icon: IosGlassNavigationIcon.trash,
        semanticLabel: 'Delete agent',
        onPressed: controller.busy ? null : remove,
        isBusy: controller.busy,
        width: buzzNavigationActionSize,
        height: buzzNavigationActionSize,
        foregroundColor: context.colors.error,
      );
    }
    return SizedBox.square(
      dimension: buzzNavigationActionSize,
      child: IconButton(
        tooltip: 'Delete agent',
        onPressed: controller.busy ? null : remove,
        color: context.colors.error,
        icon: const Icon(BuzzIcons.trash2),
      ),
    );
  }
}
