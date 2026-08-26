part of '../settings_page.dart';

class _NotificationsSection extends ConsumerWidget {
  const _NotificationsSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (defaultTargetPlatform != TargetPlatform.iOS) {
      return const SizedBox.shrink();
    }
    final community = ref.watch(activeCommunityProvider).value;
    if (community == null) return const SizedBox.shrink();

    return AppListCard(
      label: 'Notifications',
      verticalPadding: Grid.twelve,
      children: [
        AppListRow(
          key: const ValueKey('push-notifications-enabled'),
          icon: LucideIcons.bell,
          title: 'Push notifications',
          subtitle: 'Receive message notifications from this community',
          trailing: Switch.adaptive(
            value: community.pushNotificationsEnabled,
            onChanged: (enabled) => unawaited(
              ref
                  .read(communityListProvider.notifier)
                  .setPushNotificationsEnabled(community.id, enabled),
            ),
          ),
          onTap: () => unawaited(
            ref
                .read(communityListProvider.notifier)
                .setPushNotificationsEnabled(
                  community.id,
                  !community.pushNotificationsEnabled,
                ),
          ),
        ),
      ],
    );
  }
}
