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
    final active = ref.watch(activeCommunityProvider).value!;
    // The list changes in the same frame as each opt-out step, while the
    // active projection trails it by a storage read. Reading the list keeps a
    // finished opt-out from showing as failed for a frame.
    final community =
        ref
            .watch(communityListProvider)
            .value
            ?.where((candidate) => candidate.id == active.id)
            .firstOrNull ??
        active;
    final setup = ref.watch(buzzPushSetupStatusProvider);
    final capability = ref.watch(currentRelayPushDescriptorProvider);
    final hasCapability =
        !capability.isLoading &&
        !capability.hasError &&
        capability.value != null;

    final status = ref.watch(pushOptOutStatusProvider)[community.id];
    final turningOff = status == PushOptOutStatus.turningOff;
    // An opt-out the relay hasn't confirmed: one that failed in this run, or
    // one an earlier run left pending.
    final optOutFailed =
        !turningOff &&
        (status == PushOptOutStatus.failed ||
            (!community.pushNotificationsEnabled &&
                community.pushSubscriptionState.pendingTombstoneGeneration !=
                    null));
    // While unconfirmed, the switch shows the relay's last confirmed setting.
    final shownEnabled =
        !turningOff &&
        (community.pushNotificationsEnabled ||
            (optOutFailed && community.pushSubscriptionState.accepted != null));
    // Disabled while turning off, so a second tap can't start another opt-out
    // before this one ends.
    final canToggle =
        !turningOff && (hasCapability || shownEnabled || optOutFailed);
    final communities = ref.read(communityListProvider.notifier);

    void retryOptOut() => unawaited(communities.retryPushOptOut(community.id));

    void setEnabled(bool enabled) {
      if (optOutFailed && !enabled) return retryOptOut();
      unawaited(communities.setPushNotificationsEnabled(community.id, enabled));
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
                // Until the relay confirms, off is not yet true.
                subtitle: turningOff ? 'Turning off…' : null,
                // Reserve horizontal room without letting the switch's 48dp
                // layout height expand the row.
                trailing: const SizedBox(width: 60),
                onTap: canToggle ? () => setEnabled(!shownEnabled) : null,
              ),
              Positioned.fill(
                right: Grid.xs,
                child: Align(
                  alignment: Alignment.centerRight,
                  child: Switch.adaptive(
                    value: shownEnabled,
                    onChanged: canToggle ? setEnabled : null,
                  ),
                ),
              ),
            ],
          ),
        ),
        if (optOutFailed)
          // Announced when it appears; it sits outside the switch's merged
          // semantics. The wording is the spec's (rule 29): a timeout doesn't
          // prove the opt-out failed.
          Semantics(
            key: const ValueKey('push-opt-out-failed'),
            container: true,
            liveRegion: true,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                AppListRow(
                  title:
                      'Could not confirm that notifications were turned off.',
                  titleColor: context.colors.error,
                  subtitle: 'Notifications may continue.',
                ),
                // Below the text, not trailing it, so large text sizes keep
                // the warning readable.
                Align(
                  alignment: AlignmentDirectional.centerEnd,
                  child: TextButton(
                    onPressed: retryOptOut,
                    child: const Text(
                      'Retry',
                      semanticsLabel: 'Retry turning off notifications',
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
