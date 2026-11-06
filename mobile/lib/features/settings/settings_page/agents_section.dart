part of '../settings_page.dart';

class _AgentsSection extends ConsumerWidget {
  const _AgentsSection({
    this.onAgents,
    this.onCreateAgent,
  });

  final void Function(BuildContext context)? onAgents;
  final void Function(BuildContext context)? onCreateAgent;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final ownedAgents = ref.watch(ownedAgentDirectoryProvider).asData?.value;
    final bestiePubkey = ref.watch(defaultAgentProvider).asData?.value;
    final agentCount = ownedAgents?.length ?? 0;

    final String subtitle;
    if (bestiePubkey != null && bestiePubkey.isNotEmpty) {
      final names = watchIdentityNames(
        ref,
        [bestiePubkey],
        agentPubkeys: {bestiePubkey},
      );
      final bestieName = names.labelFor(bestiePubkey);
      subtitle = agentCount > 1
          ? 'Bestie: $bestieName · $agentCount agents'
          : 'Bestie: $bestieName';
    } else if (agentCount > 0) {
      subtitle = agentCount == 1 ? '1 agent' : '$agentCount agents';
    } else {
      subtitle = 'Manage your Bestie and custom agents';
    }

    return AppListCard(
      key: const ValueKey('settings-agents-options'),
      dividerIndent: Grid.xs,
      verticalPadding: Grid.twelve,
      children: [
        AppListRow(
          key: const ValueKey('settings-agents-row'),
          title: 'Agents',
          subtitle: subtitle,
          trailing: const _RowChevron(),
          onTap: () {
            unawaited(HapticFeedback.selectionClick());
            if (onAgents != null) {
              onAgents!(context);
            } else {
              unawaited(showAgents(context));
            }
          },
        ),
        AppListRow(
          key: const ValueKey('settings-create-agent-row'),
          title: 'Create agent',
          subtitle: 'Create a custom agent for this community',
          trailing: const _RowChevron(),
          onTap: () {
            unawaited(HapticFeedback.selectionClick());
            if (onCreateAgent != null) {
              onCreateAgent!(context);
            } else {
              unawaited(showAgents(context, createOnOpen: true));
            }
          },
        ),
      ],
    );
  }
}
