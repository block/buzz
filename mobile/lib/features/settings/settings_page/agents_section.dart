part of '../settings_page.dart';

class _AgentsSection extends ConsumerWidget {
  const _AgentsSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final enabled = ref.watch(rememberMentionedAgentsProvider);
    final setEnabled = ref
        .read(rememberMentionedAgentsProvider.notifier)
        .setEnabled;
    return AppListCard(
      verticalPadding: Grid.twelve,
      children: [
        MergeSemantics(
          child: AppListRow(
            key: const ValueKey('settings-remember-mentioned-agents'),
            title: 'Remember mentioned agents',
            subtitle: 'Start your next message with the agents you mentioned',
            trailing: Switch.adaptive(value: enabled, onChanged: setEnabled),
            onTap: () => setEnabled(!enabled),
          ),
        ),
      ],
    );
  }
}
