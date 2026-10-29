part of '../settings_page.dart';

bool _notificationsVisible(WidgetRef ref) {
  if (defaultTargetPlatform != TargetPlatform.iOS) return false;
  final community = ref.watch(activeCommunityProvider).value;
  if (community == null) return false;
  final setup = ref.watch(buzzPushSetupStatusProvider);
  final capability = ref.watch(currentRelayPushDescriptorProvider);
  return (!capability.isLoading &&
          !capability.hasError &&
          capability.value != null) ||
      community.pushNotificationsEnabled ||
      community.pushSubscriptionState.pendingTombstoneGeneration != null ||
      setup.hasError;
}

class _NotificationsSection extends ConsumerWidget {
  const _NotificationsSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    if (!_notificationsVisible(ref)) return const SizedBox.shrink();
    final community = ref.watch(activeCommunityProvider).value!;
    final setup = ref.watch(buzzPushSetupStatusProvider);
    final capability = ref.watch(currentRelayPushDescriptorProvider);
    final hasCapability =
        !capability.isLoading &&
        !capability.hasError &&
        capability.value != null;

    final canToggle = hasCapability || community.pushNotificationsEnabled;
    final omittedSoloChats = community.pushNotificationsEnabled
        ? ref.watch(pushSubscriptionSyncProvider).value ?? 0
        : 0;

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
        if (setup.hasError)
          AppListRow(
            key: const ValueKey('push-setup-retry'),
            title: 'Notification setup failed',
            subtitle: 'You can keep using Buzz. Tap to retry notifications.',
            onTap: () => retryBuzzPushSetup(ref),
          ),
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
        if (omittedSoloChats > 0)
          AppListRow(
            key: const ValueKey('push-solo-coverage'),
            title: 'Older Solo chats notify only on mention',
            subtitle: omittedSoloChats == 1
                ? '1 chat is past this relay’s notification limit.'
                : '$omittedSoloChats chats are past this relay’s notification limit.',
          ),
      ],
    );
  }
}
