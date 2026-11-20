part of '../settings_page.dart';

bool _notificationsVisible(WidgetRef ref) {
  if (defaultTargetPlatform != TargetPlatform.iOS &&
      defaultTargetPlatform != TargetPlatform.android) {
    return false;
  }
  final community = ref.watch(activeCommunityProvider).value;
  if (community == null) return false;
  // Without a gateway an Android build never enrolls, so there is nothing to
  // switch on. A saved opt-in (or an unfinished opt-out) still shows the row:
  // turning it off is the only move left, and the row goes inert once off.
  // (iOS keeps a saved opt-in visible the same way.)
  if (defaultTargetPlatform == TargetPlatform.android &&
      !Env.pushGatewayConfigured) {
    return community.pushNotificationsEnabled ||
        community.pushSubscriptionState.pendingTombstoneGeneration != null;
  }
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
        ValueListenableBuilder<String?>(
          valueListenable: pushUnavailableReason,
          builder: (context, reason, _) => reason == null
              ? const SizedBox.shrink()
              : const AppListRow(
                  key: ValueKey('push-unavailable'),
                  title: 'Push is unavailable on this phone',
                  subtitle:
                      'You can keep using Buzz. New messages appear when you '
                      'open the app.',
                ),
        ),
        MergeSemantics(
          key: const ValueKey('push-notifications-setting'),
          child: Stack(
            alignment: Alignment.centerRight,
            children: [
              AppListRow(
                key: const ValueKey('push-notifications-enabled'),
                title: 'Notifications',
                subtitle:
                    !community.pushNotificationsEnabled &&
                        community
                                .pushSubscriptionState
                                .pendingTombstoneGeneration !=
                            null
                    ? 'Turning off. Notifications may continue until this '
                          'finishes.'
                    : null,
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
