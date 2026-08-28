part of '../settings_page.dart';

class _NotificationsSection extends ConsumerWidget {
  const _NotificationsSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final alertsOn = ref.watch(notificationPreferencesProvider).allMessages;

    return AppListCard(
      label: 'Notifications',
      verticalPadding: Grid.twelve,
      children: [
        AppListRow(
          icon: LucideIcons.bell,
          title: 'All messages',
          subtitle: 'Sound and notify when someone posts in a channel.',
          trailing: Switch.adaptive(
            value: alertsOn,
            onChanged: (value) {
              unawaited(
                ref
                    .read(notificationPreferencesProvider.notifier)
                    .setAllMessages(value),
              );
            },
          ),
        ),
      ],
    );
  }
}
