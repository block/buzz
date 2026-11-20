part of '../settings_page.dart';

bool _notificationsVisible(WidgetRef ref) =>
    defaultTargetPlatform == TargetPlatform.iOS &&
    ref.watch(activeCommunityProvider).value != null;

bool _pushNotificationsVisible(WidgetRef ref) {
  if (!_notificationsVisible(ref)) return false;
  final community = ref.watch(activeCommunityProvider).value!;
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
    final showPush = _pushNotificationsVisible(ref);
    final mentionsOnly = ref.watch(appBadgeMentionsOnlyProvider);
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

    void setMentionsOnly(bool value) {
      unawaited(ref.read(appBadgeMentionsOnlyProvider.notifier).set(value));
    }

    return AppListCard(
      dividerIndent: Grid.xs,
      verticalPadding: Grid.twelve,
      children: [
        if (showPush && setup.hasError)
          AppListRow(
            key: const ValueKey('push-setup-retry'),
            title: 'Notification setup failed',
            subtitle: 'You can keep using Buzz. Tap to retry notifications.',
            onTap: () => retryBuzzPushSetup(ref),
          ),
        if (showPush)
          _SwitchRow(
            key: const ValueKey('push-notifications-setting'),
            rowKey: const ValueKey('push-notifications-enabled'),
            title: 'Notifications',
            value: community.pushNotificationsEnabled,
            onChanged: canToggle ? setEnabled : null,
          ),
        _SwitchRow(
          key: const ValueKey('app-badge-mentions-only'),
          rowKey: const ValueKey('app-badge-mentions-only-row'),
          title: 'Badge only mentions and DMs',
          value: mentionsOnly,
          onChanged: setMentionsOnly,
        ),
      ],
    );
  }
}

class _SwitchRow extends StatelessWidget {
  const _SwitchRow({
    super.key,
    required this.rowKey,
    required this.title,
    required this.value,
    required this.onChanged,
  });

  final Key rowKey;
  final String title;
  final bool value;
  final ValueChanged<bool>? onChanged;

  @override
  Widget build(BuildContext context) {
    final onChanged = this.onChanged;
    return MergeSemantics(
      child: Stack(
        alignment: Alignment.centerRight,
        children: [
          AppListRow(
            key: rowKey,
            title: title,
            // Reserve horizontal room without letting the switch's 48dp
            // layout height expand this single-line row.
            trailing: const SizedBox(width: 60),
            onTap: onChanged == null ? null : () => onChanged(!value),
          ),
          Positioned.fill(
            right: Grid.xs,
            child: Align(
              alignment: Alignment.centerRight,
              child: Switch.adaptive(value: value, onChanged: onChanged),
            ),
          ),
        ],
      ),
    );
  }
}
