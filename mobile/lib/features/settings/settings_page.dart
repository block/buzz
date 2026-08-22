import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:package_info_plus/package_info_plus.dart';

import '../../shared/auth/auth.dart';
import '../../shared/clipboard_utils.dart';
import '../../shared/community/community_membership_provider.dart';
import '../../shared/relay/relay.dart';
import '../pairing/pairing_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import '../../shared/widgets/modal_presentation.dart';
import 'accent_picker_page.dart';
import 'theme_picker_page.dart';

part 'settings_page/appearance_section.dart';
part 'settings_page/community_section.dart';
part 'settings_page/connection_section.dart';

class SettingsPage extends HookConsumerWidget {
  /// Creates the settings page.
  const SettingsPage({
    super.key,
    required this.profileHeader,
    required this.invitePageBuilder,
    required this.identityRecoveryPageBuilder,
  });

  /// Header widget displayed at the top of settings.
  final Widget profileHeader;

  /// Builds the community-invite page pushed from the invite settings row.
  final WidgetBuilder invitePageBuilder;

  /// Builds the identity-recovery page pushed from the recovery settings row.
  final WidgetBuilder identityRecoveryPageBuilder;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final packageInfoFuture = useMemoized(() => PackageInfo.fromPlatform());
    final packageInfo = useFuture(packageInfoFuture);
    final topSectionHeight = frostedAppBarHeight(
      context,
      bottomHeight: Grid.xxs,
    );

    return FrostedScaffold(
      backgroundColor: context.colors.surface,
      appBar: FrostedAppBar(
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
                  icon: const Icon(LucideIcons.x),
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
                _CommunitySection(invitePageBuilder: invitePageBuilder),
                const _AppearanceSection(),
                _ConnectionSection(
                  identityRecoveryPageBuilder: identityRecoveryPageBuilder,
                ),
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
