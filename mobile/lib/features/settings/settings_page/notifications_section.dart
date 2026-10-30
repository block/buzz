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
    final setup = ref.watch(buzzPushSetupStatusProvider);
    final capability = ref.watch(currentRelayPushDescriptorProvider);
    final hasCapability =
        !capability.isLoading &&
        !capability.hasError &&
        capability.value != null;
    final optOutPending =
        community.pushSubscriptionState.pendingTombstoneGeneration != null;
    if (!hasCapability &&
        !community.pushNotificationsEnabled &&
        !optOutPending &&
        !setup.hasError) {
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
        if (setup.hasError)
          AppListRow(
            key: const ValueKey('push-setup-retry'),
            title: 'Notification setup failed',
            subtitle: 'You can keep using Buzz. Tap to retry notifications.',
            onTap: () => retryBuzzPushSetup(ref),
          ),
        _PushSwitchRow(
          name: 'push-notifications',
          title: 'Notifications',
          value: community.pushNotificationsEnabled,
          onChanged: canToggle ? setEnabled : null,
        ),
        if (community.pushNotificationsEnabled)
          _PushSwitchRow(
            name: 'push-mute-agents',
            title: 'Mute agents',
            subtitle: 'Only notify for messages from people.',
            value: community.pushMuteAgents,
            onChanged: (muted) => unawaited(
              ref
                  .read(communityListProvider.notifier)
                  .setPushMuteAgents(community.id, muted),
            ),
          ),
      ],
    );
  }
}

class _PushSwitchRow extends StatelessWidget {
  const _PushSwitchRow({
    required this.name,
    required this.title,
    this.subtitle,
    required this.value,
    required this.onChanged,
  });

  final String name;
  final String title;
  final String? subtitle;
  final bool value;
  final ValueChanged<bool>? onChanged;

  @override
  Widget build(BuildContext context) => MergeSemantics(
    key: ValueKey('$name-setting'),
    child: Stack(
      alignment: Alignment.centerRight,
      children: [
        AppListRow(
          key: ValueKey('$name-enabled'),
          title: title,
          subtitle: subtitle,
          // Reserve horizontal room without letting the switch's 48dp
          // layout height expand this single-line row.
          trailing: const SizedBox(width: 60),
          onTap: onChanged == null ? null : () => onChanged!(!value),
        ),
        Positioned.fill(
          right: Grid.xs,
          child: Align(
            alignment: Alignment.centerRight,
            child: Switch.adaptive(
              key: ValueKey('$name-switch'),
              value: value,
              onChanged: onChanged,
            ),
          ),
        ),
      ],
    ),
  );
}
