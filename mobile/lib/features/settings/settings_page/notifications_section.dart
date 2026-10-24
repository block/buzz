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
    final capability = ref.watch(currentRelayPushDescriptorProvider);
    final hasCapability =
        !capability.isLoading &&
        !capability.hasError &&
        capability.value != null;
    final optOutPending =
        community.pushSubscriptionState.pendingTombstoneGeneration != null;
    if (!hasCapability &&
        !community.pushNotificationsEnabled &&
        !optOutPending) {
      return const SizedBox.shrink();
    }
    final canToggle = hasCapability || community.pushNotificationsEnabled;

    void setEnabled(bool enabled) {
      unawaited(
        ref
            .read(communityListProvider.notifier)
            .setPushNotificationsEnabled(community.id, enabled),
      );
    }

    return AppListCard(
      verticalPadding: Grid.twelve,
      children: [
        MergeSemantics(
          key: const ValueKey('push-notifications-setting'),
          child: Stack(
            alignment: Alignment.centerRight,
            children: [
              AppListRow(
                key: const ValueKey('push-notifications-enabled'),
                title: 'Notifications',
                // Reserve horizontal room without letting the switch's 48dp
                // layout height expand this single-line row.
                trailing: const SizedBox(width: 60),
                onTap: canToggle
                    ? () => setEnabled(!community.pushNotificationsEnabled)
                    : null,
              ),
              Positioned.fill(
                right: Grid.xs,
                child: Align(
                  alignment: Alignment.centerRight,
                  child: Switch.adaptive(
                    value: community.pushNotificationsEnabled,
                    onChanged: canToggle ? setEnabled : null,
                  ),
                ),
              ),
            ],
          ),
        ),
      ],
    );
  }
}
