import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:package_info_plus/package_info_plus.dart';

import '../../shared/auth/auth.dart';
import '../../shared/clipboard_utils.dart';
import '../../shared/success_haptic.dart';
import '../../shared/push/push_bridge.dart';
import '../../shared/push/push_relay_capability_provider.dart';
import '../../shared/relay/relay.dart';
import '../../shared/utils/string_utils.dart';
import '../pairing/pairing_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import '../../shared/widgets/immediate_page_route.dart';

part 'settings_page/profile_section.dart';
part 'settings_page/status_section.dart';
part 'settings_page/connection_section.dart';
part 'settings_page/notifications_section.dart';

Widget _emptyProfileEditPage(BuildContext context) => const SizedBox.shrink();

class SettingsPage extends HookConsumerWidget {
  /// Creates the settings page.
  const SettingsPage({
    super.key,
    required this.profileHeader,
    required this.identityRecoveryPageBuilder,
    this.profileEditPageBuilder = _emptyProfileEditPage,
    this.onSetStatus,
    this.onEditDisplayName,
    this.onEditProfileDescription,
  });

  /// Header widget displayed at the top of settings.
  final Widget profileHeader;

  /// Builds the identity-recovery page pushed from the recovery settings row.
  final WidgetBuilder identityRecoveryPageBuilder;

  /// Builds the current-user profile editor opened from the Photo row.
  final WidgetBuilder profileEditPageBuilder;

  /// Opens the current-user status editor.
  final void Function(BuildContext context)? onSetStatus;

  /// Opens the display-name editor from the settings section.
  final Future<void> Function(BuildContext context)? onEditDisplayName;

  /// Opens the profile-description editor from the settings section.
  final Future<void> Function(BuildContext context)? onEditProfileDescription;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final packageInfoFuture = useMemoized(() => PackageInfo.fromPlatform());
    final packageInfo = useFuture(packageInfoFuture);
    final topSectionHeight = frostedAppBarHeight(
      context,
      bottomHeight: Grid.xxs,
    );

    return FrostedScaffold(
      useUtilitySurfaceTheme: true,
      appBar: FrostedAppBar(
        nativeTitle: 'Settings',
        nativeLeading: IosNavigationAction(
          label: 'Close settings',
          symbol: 'xmark',
          onPressed: () => Navigator.of(context).pop(),
        ),
        automaticallyImplyLeading: false,
        horizontalInset: Grid.gutter,
        showBottomDivider: false,
        leading: Theme.of(context).platform == TargetPlatform.iOS
            ? IosGlassNavigationButton(
                key: const ValueKey('settings-ios-glass-close'),
                icon: IosGlassNavigationIcon.close,
                semanticLabel: 'Close settings',
                onPressed: () {
                  unawaited(HapticFeedback.lightImpact());
                  Navigator.of(context).pop();
                },
                foregroundColor: navigationPrimaryForeground(context),
              )
            : SizedBox(
                width: Grid.xl,
                height: Grid.xl,
                child: IconButton(
                  tooltip: 'Close settings',
                  onPressed: () {
                    unawaited(HapticFeedback.lightImpact());
                    Navigator.of(context).pop();
                  },
                  color: navigationPrimaryForeground(context),
                  icon: const Icon(BuzzIcons.x),
                ),
              ),
        bottomHeight: Grid.xxs,
        bottom: const SizedBox.expand(),
      ),
      body: Column(
        children: [
          Expanded(
            child: ListView(
              padding: EdgeInsets.only(top: topSectionHeight, bottom: Grid.xs),
              children: [
                profileHeader,
                _StatusSection(onSetStatus: onSetStatus),
                _ProfileSection(
                  profileEditPageBuilder: profileEditPageBuilder,
                  onEditDisplayName: onEditDisplayName,
                  onEditProfileDescription: onEditProfileDescription,
                ),
                const _NotificationsSection(),
                _ConnectionSection(
                  identityRecoveryPageBuilder: identityRecoveryPageBuilder,
                ),
              ],
            ),
          ),
          if (packageInfo.hasData)
            _VersionFooter(
              version: packageInfo.data!.version,
              buildNumber: packageInfo.data!.buildNumber,
            ),
        ],
      ),
    );
  }
}

class _VersionFooter extends StatelessWidget {
  const _VersionFooter({required this.version, required this.buildNumber});

  final String version;
  final String buildNumber;

  @override
  Widget build(BuildContext context) {
    return SafeArea(
      top: false,
      child: Padding(
        padding: const EdgeInsets.only(bottom: Grid.xs, top: Grid.xxs),
        child: Center(
          child: Text(
            buildNumber.isEmpty ? 'v$version' : 'v$version ($buildNumber)',
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
      BuzzIcons.chevronRight,
      size: 18,
      color: context.colors.onSurfaceVariant,
    );
  }
}
