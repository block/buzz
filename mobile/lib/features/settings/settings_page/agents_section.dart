part of '../settings_page.dart';

class _AgentsSection extends ConsumerWidget {
  const _AgentsSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final automaticallyMentionAgents = ref.watch(autoMentionAgentsProvider);

    Future<void> setAutomaticallyMentionAgents(bool enabled) async {
      try {
        await ref.read(autoMentionAgentsProvider.notifier).setEnabled(enabled);
      } catch (_) {
        if (!context.mounted) return;
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
            content: Text('Could not save the automatic mention setting.'),
          ),
        );
      }
    }

    return AppListCard(
      label: 'Agents',
      verticalPadding: Grid.twelve,
      children: [
        AppListRow(
          key: const ValueKey('settings-automatic-agent-mentions'),
          icon: LucideIcons.bot,
          title: 'Automatically mention agents',
          subtitle: 'Address selected agents in thread replies',
          trailing: Switch.adaptive(
            key: const ValueKey('settings-automatic-agent-mentions-switch'),
            value: automaticallyMentionAgents,
            onChanged: (enabled) =>
                unawaited(setAutomaticallyMentionAgents(enabled)),
          ),
          onTap: () => unawaited(
            setAutomaticallyMentionAgents(!automaticallyMentionAgents),
          ),
        ),
      ],
    );
  }
}
