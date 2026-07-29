part of '../settings_page.dart';

/// Device-local message display preferences.
class _MessagesSection extends ConsumerWidget {
  const _MessagesSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final showJoinLeave = ref.watch(showJoinLeaveMessagesProvider);

    return AppListCard(
      label: 'Messages',
      children: [
        AppListRow(
          icon: LucideIcons.userPlus,
          title: 'Show join and leave messages',
          trailing: Switch(
            value: showJoinLeave,
            onChanged: (enabled) => ref
                .read(showJoinLeaveMessagesProvider.notifier)
                .setEnabled(enabled),
          ),
          onTap: () => ref
              .read(showJoinLeaveMessagesProvider.notifier)
              .setEnabled(!showJoinLeave),
        ),
      ],
    );
  }
}
