part of '../settings_page.dart';

class _AgentsSection extends ConsumerWidget {
  const _AgentsSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final mode = ref.watch(unaddressedChannelAgentModeProvider);

    return AppListCard(
      label: 'Agents',
      children: [
        AppListRow(
          icon: LucideIcons.bot,
          title: 'Unaddressed channel messages',
          value: mode == UnaddressedChannelAgentMode.mentionsOnly
              ? 'Mentions only'
              : 'Notify all channel agents',
          trailing: const _RowChevron(),
          onTap: () => _showUnaddressedModeSheet(context),
        ),
      ],
    );
  }
}

void _showUnaddressedModeSheet(BuildContext context) {
  showModalBottomSheet<void>(
    context: context,
    showDragHandle: true,
    builder: (_) => const _UnaddressedModeSheet(),
  );
}

class _UnaddressedModeSheet extends ConsumerWidget {
  const _UnaddressedModeSheet();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final mode = ref.watch(unaddressedChannelAgentModeProvider);
    final notifier = ref.read(unaddressedChannelAgentModeProvider.notifier);

    return SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              Grid.xs,
              Grid.gutter,
              Grid.xxs,
            ),
            child: Text(
              'Unaddressed channel messages',
              style: context.textTheme.titleMedium,
            ),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              0,
              Grid.gutter,
              Grid.sm,
            ),
            child: Text(
              'When you post in a channel without @mentioning anyone, choose who is notified.',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ),
          for (final option in const [
            (
              UnaddressedChannelAgentMode.allChannelAgents,
              'Notify all channel agents',
            ),
            (UnaddressedChannelAgentMode.mentionsOnly, 'Mentions only'),
          ])
            ListTile(
              title: Text(option.$2),
              trailing: mode == option.$1
                  ? Icon(LucideIcons.check, color: context.colors.primary)
                  : null,
              onTap: () {
                notifier.setMode(option.$1);
                Navigator.of(context).pop();
              },
            ),
          const SizedBox(height: Grid.sm),
        ],
      ),
    );
  }
}
