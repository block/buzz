import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:package_info_plus/package_info_plus.dart';

import '../../shared/auth/auth.dart';
import '../../shared/clipboard_utils.dart';
import '../../shared/notifications/notifications.dart';
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import 'accent_picker_page.dart';
import 'theme_picker_page.dart';

part 'settings_page/appearance_section.dart';
part 'settings_page/connection_section.dart';

class SettingsPage extends HookConsumerWidget {
  const SettingsPage({super.key, required this.profileHeader});

  final Widget profileHeader;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final packageInfoFuture = useMemoized(() => PackageInfo.fromPlatform());
    final packageInfo = useFuture(packageInfoFuture);

    return FrostedScaffold(
      appBar: const FrostedAppBar(title: Text('Settings')),
      body: Column(
        children: [
          Expanded(
            child: ListView(
              padding: EdgeInsets.only(
                top: frostedAppBarHeight(context),
                bottom: Grid.xs,
              ),
              children: [
                profileHeader,
                const _AppearanceSection(),
                if (defaultTargetPlatform == TargetPlatform.android)
                  const _NotificationSettingsSection(),
                const _ConnectionSection(),
                const _RemoveCommunitySection(),
              ],
            ),
          ),
          if (packageInfo.hasData)
            _VersionFooter(version: packageInfo.data!.version),
        ],
      ),
    );
  }
}

class _NotificationSettingsSection extends ConsumerWidget {
  const _NotificationSettingsSection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final settings = ref.watch(notificationSettingsProvider);
    final sessionStatus = ref.watch(
      relaySessionProvider.select((session) => session.status),
    );
    final isConnected = sessionStatus == SessionStatus.connected;
    final isBlocked =
        settings.permission == AndroidNotificationPermission.denied;
    final selectedChannelDisabled =
        (settings.priorityEnabled && !settings.priorityChannelEnabled) ||
        (settings.activityEnabled && !settings.activityChannelEnabled);
    final status = settings.isRequesting
        ? 'Waiting for Android…'
        : switch ((
            settings.alertsEnabled,
            isBlocked,
            isConnected,
            selectedChannelDisabled,
          )) {
            (_, true, _, _) => 'Blocked',
            (false, _, _, _) => 'Off',
            (true, _, false, _) => 'Paused',
            (true, _, true, true) => 'Limited',
            _ => 'On',
          };

    return AppListCard(
      label: 'Notifications',
      children: [
        AppListRow(
          icon: LucideIcons.bell,
          title: 'Alerts',
          subtitle: status == 'Waiting for Android…'
              ? 'Waiting for Android…'
              : status == 'Blocked'
              ? 'Android has blocked Buzz notifications.'
              : status == 'Paused'
              ? 'Paused while Buzz reconnects.'
              : status == 'Limited'
              ? 'A notification category is disabled in Android settings.'
              : 'Alerts require a live Buzz connection.',
          value: status,
          trailing: isBlocked
              ? TextButton(
                  onPressed: () => ref
                      .read(notificationSettingsProvider.notifier)
                      .openSettings(),
                  child: const Text('Android settings'),
                )
              : Switch.adaptive(
                  value: settings.alertsEnabled,
                  onChanged: settings.isRequesting
                      ? null
                      : (enabled) => ref
                            .read(notificationSettingsProvider.notifier)
                            .setAlertsEnabled(enabled),
                ),
        ),
        AppListRow(
          icon: LucideIcons.circleAlert,
          title: 'Mentions & direct messages',
          subtitle:
              'Alert when someone mentions you or sends you a direct message.',
          trailing: Switch.adaptive(
            value: settings.priorityEnabled,
            onChanged: settings.alertsEnabled
                ? (enabled) => ref
                      .read(notificationSettingsProvider.notifier)
                      .setPriorityEnabled(enabled)
                : null,
          ),
        ),
        AppListRow(
          icon: LucideIcons.activity,
          title: 'Channel activity',
          subtitle: 'Alert for new top-level messages in unmuted channels.',
          trailing: Switch.adaptive(
            value: settings.activityEnabled,
            onChanged: settings.alertsEnabled
                ? (enabled) => ref
                      .read(notificationSettingsProvider.notifier)
                      .setActivityEnabled(enabled)
                : null,
          ),
        ),
        AppListRow(
          icon: LucideIcons.eye,
          title: 'Message previews',
          subtitle: settings.previewsEnabled
              ? 'Show sender, channel, and message text in notifications.'
              : 'Hide sender, channel, and message text in notifications.',
          trailing: Switch.adaptive(
            value: settings.previewsEnabled,
            onChanged: settings.alertsEnabled
                ? (enabled) => ref
                      .read(notificationSettingsProvider.notifier)
                      .setPreviewsEnabled(enabled)
                : null,
          ),
        ),
        const AppListRow(
          icon: LucideIcons.wifiOff,
          title: 'Background alerts',
          value: 'Not available',
          subtitle:
              'Buzz does not use Google services. Alerts require a live Buzz connection.',
        ),
      ],
    );
  }
}

class _VersionFooter extends StatelessWidget {
  const _VersionFooter({required this.version});

  final String version;

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      top: false,
      child: Padding(
        padding: const EdgeInsets.only(bottom: Grid.xs, top: Grid.xxs),
        child: Center(
          child: Text(
            'v$version',
            style: context.textTheme.bodySmall?.copyWith(
              color: context.colors.onSurfaceVariant.withValues(alpha: 0.6),
            ),
          ),
        ),
      ),
    );
  }
}

/// Trailing affordance shared by the rows that push a picker page.
class _RowChevron extends StatelessWidget {
  const _RowChevron();

  @override
  Widget build(BuildContext context) {
    return Icon(
      LucideIcons.chevronRight,
      size: 18,
      color: context.colors.onSurfaceVariant,
    );
  }
}
