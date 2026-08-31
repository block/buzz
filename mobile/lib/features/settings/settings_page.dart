import 'dart:async';

import 'package:flutter/foundation.dart';
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
import '../../shared/push/push_bridge.dart';
import '../../shared/relay/relay.dart';
import '../pairing/pairing_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import '../../shared/widgets/ios_glass_navigation_action.dart';
import '../../shared/widgets/immediate_page_route.dart';
import '../../shared/widgets/modal_presentation.dart';
import 'theme_picker_page.dart';

part 'settings_page/community_section.dart';
part 'settings_page/connection_section.dart';
part 'settings_page/notifications_section.dart';

Widget _emptyProfileEditPage(BuildContext context) => const SizedBox.shrink();

enum _ProfileEditAction { displayName, description, photo }

class SettingsPage extends HookConsumerWidget {
  /// Creates the settings page.
  const SettingsPage({
    super.key,
    required this.profileHeader,
    required this.invitePageBuilder,
    required this.identityRecoveryPageBuilder,
    this.profileEditPageBuilder = _emptyProfileEditPage,
    this.onEditDisplayName,
    this.onEditProfileDescription,
  });

  /// Header widget displayed at the top of settings.
  final Widget profileHeader;

  /// Builds the community-invite page pushed from the invite settings row.
  final WidgetBuilder invitePageBuilder;

  /// Builds the identity-recovery page pushed from the recovery settings row.
  final WidgetBuilder identityRecoveryPageBuilder;

  /// Builds the current-user profile editor opened from the top action.
  final WidgetBuilder profileEditPageBuilder;

  /// Opens the display-name editor after the Edit Profile sheet closes.
  final Future<void> Function(BuildContext context)? onEditDisplayName;

  /// Opens the profile-description editor after the Edit Profile sheet closes.
  final Future<void> Function(BuildContext context)? onEditProfileDescription;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final packageInfoFuture = useMemoized(() => PackageInfo.fromPlatform());
    final packageInfo = useFuture(packageInfoFuture);
    final topSectionHeight = frostedAppBarHeight(
      context,
      bottomHeight: Grid.xxs,
    );

    Future<void> showEditProfileSheet() async {
      final action = await showBuzzModalBottomSheet<_ProfileEditAction>(
        context: context,
        title: 'Edit profile',
        builder: (sheetContext) => SafeArea(
          top: false,
          child: Padding(
            key: const ValueKey('edit-profile-sheet-content'),
            padding: const EdgeInsets.only(bottom: Grid.xs),
            // AppListCard normally fills the height offered by a page section.
            // Give it unbounded vertical space here so this compact action sheet
            // hugs its three rows instead of filling the modal height cap.
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                AppListCard(
                  key: const ValueKey('edit-profile-options'),
                  dividerIndent: Grid.xs,
                  verticalPadding: 0,
                  children: [
                    AppListRow(
                      key: const ValueKey('edit-profile-display-name'),
                      title: 'Display name',
                      trailing: const _RowChevron(),
                      onTap: () => Navigator.pop(
                        sheetContext,
                        _ProfileEditAction.displayName,
                      ),
                    ),
                    AppListRow(
                      key: const ValueKey('edit-profile-description'),
                      title: 'Profile description',
                      trailing: const _RowChevron(),
                      onTap: () => Navigator.pop(
                        sheetContext,
                        _ProfileEditAction.description,
                      ),
                    ),
                    AppListRow(
                      key: const ValueKey('edit-profile-photo'),
                      title: 'Photo',
                      trailing: const _RowChevron(),
                      onTap: () =>
                          Navigator.pop(sheetContext, _ProfileEditAction.photo),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ),
      );
      if (!context.mounted || action == null) return;
      unawaited(HapticFeedback.selectionClick());
      switch (action) {
        case _ProfileEditAction.displayName:
          await onEditDisplayName?.call(context);
          break;
        case _ProfileEditAction.description:
          await onEditProfileDescription?.call(context);
          break;
        case _ProfileEditAction.photo:
          await Navigator.of(
            context,
          ).push(immediatePageRoute<void>(builder: profileEditPageBuilder));
          break;
      }
    }

    return FrostedScaffold(
      useUtilitySurfaceTheme: true,
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
        actions: [
          if (Theme.of(context).platform == TargetPlatform.iOS)
            IosGlassNavigationAction(
              key: const ValueKey('settings-edit-profile'),
              label: 'Edit',
              foregroundColor: navigationPrimaryForeground(context),
              onPressed: () => unawaited(showEditProfileSheet()),
            )
          else
            Material(
              key: const ValueKey('settings-edit-profile'),
              color: context.colors.surfaceContainerHighest,
              borderRadius: BorderRadius.circular(Radii.full),
              clipBehavior: Clip.antiAlias,
              child: InkWell(
                onTap: () => unawaited(showEditProfileSheet()),
                child: Padding(
                  padding: const EdgeInsets.symmetric(
                    horizontal: Grid.xs,
                    vertical: Grid.xxs,
                  ),
                  child: Text(
                    'Edit',
                    style: context.textTheme.labelLarge?.copyWith(
                      color: context.colors.onSurface,
                      fontWeight: FontWeight.w600,
                    ),
                  ),
                ),
              ),
            ),
        ],
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
                const _NotificationsSection(),
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
