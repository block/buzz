part of '../settings_page.dart';

/// Formats a [TimeOfDay] as a short localized label like "10:30 PM".
String _formatTimeOfDay(BuildContext context, TimeOfDay t) {
  return t.format(context);
}

/// Notifications settings section: push notification master switch and
/// quiet-hours schedule.
///
/// Quiet hours suppress incoming push notifications during a configurable
/// time window (defaulting to 22:00–08:00). The window may wrap past
/// midnight.
class _NotificationsSection extends ConsumerWidget {
  const _NotificationsSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final prefs = ref.watch(notificationPrefsProvider);

    return AppListCard(
      label: 'Notifications',
      children: [
        AppListRow(
          icon: LucideIcons.bell,
          title: 'Push notifications',
          trailing: _NotificationSwitch(
            value: prefs.globalEnabled,
            onChanged: ref
                .read(notificationPrefsProvider.notifier)
                .setGlobalEnabled,
          ),
        ),
        AppListRow(
          icon: LucideIcons.moonStar,
          title: 'Quiet hours',
          value: prefs.quietHoursEnabled ? 'On' : 'Off',
          trailing: const _RowChevron(),
          onTap: () => _showQuietHoursSheet(context, ref),
        ),
        if (prefs.quietHoursEnabled) ...[
          AppListRow(
            icon: LucideIcons.clock,
            title: 'Starts',
            value: _formatTimeOfDay(context, prefs.effectiveStart),
            trailing: const _RowChevron(),
            onTap: () => _pickQuietHoursTime(
              context,
              ref,
              isStart: true,
              current: prefs.effectiveStart,
            ),
          ),
          AppListRow(
            icon: LucideIcons.clock,
            title: 'Ends',
            value: _formatTimeOfDay(context, prefs.effectiveEnd),
            trailing: const _RowChevron(),
            onTap: () => _pickQuietHoursTime(
              context,
              ref,
              isStart: false,
              current: prefs.effectiveEnd,
            ),
          ),
        ],
      ],
    );
  }

  void _showQuietHoursSheet(BuildContext context, WidgetRef ref) {
    final prefs = ref.read(notificationPrefsProvider);
    showModalBottomSheet<void>(
      context: context,
      showDragHandle: true,
      builder: (_) => _QuietHoursToggleSheet(
        enabled: prefs.quietHoursEnabled,
        onToggle: ref
            .read(notificationPrefsProvider.notifier)
            .setQuietHoursEnabled,
      ),
    );
  }

  Future<void> _pickQuietHoursTime(
    BuildContext context,
    WidgetRef ref, {
    required bool isStart,
    required TimeOfDay current,
  }) async {
    final picked = await showTimePicker(
      context: context,
      initialTime: current,
      helpText: isStart ? 'Quiet hours start' : 'Quiet hours end',
    );
    if (picked == null) return;
    final notifier = ref.read(notificationPrefsProvider.notifier);
    if (isStart) {
      notifier.setQuietHoursStart(picked);
    } else {
      notifier.setQuietHoursEnd(picked);
    }
  }
}

/// A styled switch that matches the app's color scheme.
class _NotificationSwitch extends StatelessWidget {
  const _NotificationSwitch({required this.value, required this.onChanged});

  final bool value;
  final ValueChanged<bool> onChanged;

  @override
  Widget build(BuildContext context) {
    return Switch.adaptive(
      value: value,
      activeThumbColor: context.colors.primary,
      onChanged: onChanged,
    );
  }
}

/// Bottom sheet for toggling quiet hours on/off, with a brief description.
class _QuietHoursToggleSheet extends StatelessWidget {
  const _QuietHoursToggleSheet({required this.enabled, required this.onToggle});

  final bool enabled;
  final ValueChanged<bool> onToggle;

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.start,
        children: [
          Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              0,
              Grid.gutter,
              Grid.xxs,
            ),
            child: Text('Quiet hours', style: context.textTheme.titleMedium),
          ),
          Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.gutter,
              0,
              Grid.gutter,
              Grid.xs,
            ),
            child: Text(
              'Suppress push notifications during a scheduled time window. '
              'The window may wrap past midnight.',
              style: context.textTheme.bodySmall?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ),
          AppListRow(
            icon: LucideIcons.moonStar,
            title: 'Enabled',
            trailing: _NotificationSwitch(value: enabled, onChanged: onToggle),
          ),
          const SizedBox(height: Grid.xxs),
        ],
      ),
    );
  }
}
