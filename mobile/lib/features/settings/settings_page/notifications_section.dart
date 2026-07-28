part of '../settings_page.dart';

class _NotificationsSection extends ConsumerWidget {
  const _NotificationsSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final enabled = ref.watch(agentLiveUpdatesEnabledProvider);

    void setEnabled(bool value) {
      ref.read(agentLiveUpdatesEnabledProvider.notifier).setEnabled(value);
    }

    return AppListCard(
      label: 'Notifications',
      children: [
        AppListRow(
          icon: LucideIcons.bot,
          title: 'Agent activity',
          subtitle: 'Show live progress while agents are working',
          trailing: Switch(value: enabled, onChanged: setEnabled),
          onTap: () => setEnabled(!enabled),
        ),
      ],
    );
  }
}
