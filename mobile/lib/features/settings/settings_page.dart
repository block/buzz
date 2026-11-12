import 'package:buzz/shared/widgets/feedback_snackbar.dart';
import 'package:buzz/shared/widgets/adaptive_conversation_pane.dart';

import 'dart:async';

import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:flutter/foundation.dart';
import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:package_info_plus/package_info_plus.dart';
import 'package:url_launcher/url_launcher.dart';

import '../../shared/auth/auth.dart';
import '../../shared/identity_transfer/nsec_export_sheet.dart';
import '../../shared/clipboard_utils.dart';
import '../../shared/hosted/hosted_environment.dart';
import '../../shared/hosted/organization_environment.dart';
import '../../shared/hosted/organization_sign_out.dart';
import '../../shared/push/push_relay_capability_provider.dart';
import '../../shared/push/push_bootstrap.dart';
import '../../shared/relay/relay.dart';
import '../../shared/utils/string_utils.dart';
import '../pairing/pairing_provider.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/app_list.dart';
import '../../shared/widgets/app_list_card.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import '../../shared/widgets/duo_toolbar.dart';
import '../../shared/widgets/bottom_up_page_route.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import '../../shared/widgets/immediate_page_route.dart';

part 'settings_page/profile_section.dart';
part 'settings_page/status_section.dart';
part 'settings_page/connection_section.dart';
part 'settings_page/notifications_section.dart';
part 'settings_page/builderlab_account_section.dart';

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
    final strip = FoldGeometry.of(context).strip;
    final avatarFirst =
        defaultTargetPlatform == TargetPlatform.iOS && strip != null;
    final topSectionHeight = avatarFirst
        ? (usesConversationPanes(context, strip: strip) ? 0.0 : Grid.sm)
        : frostedAppBarHeight(context, bottomHeight: Grid.xxs);

    final wide = usesConversationPanes(context, strip: strip);

    final sections = <(String, Widget)>[
      ('Status', _StatusSection(onSetStatus: onSetStatus)),
      (
        'Profile',
        _ProfileSection(
          profileEditPageBuilder: profileEditPageBuilder,
          onEditDisplayName: onEditDisplayName,
          onEditProfileDescription: onEditProfileDescription,
        ),
      ),
      if (_notificationsVisible(ref))
        ('Notifications', const _NotificationsSection()),
      (
        'Connection',
        _ConnectionSection(
          identityRecoveryPageBuilder: identityRecoveryPageBuilder,
        ),
      ),
      if (HostedEnvironment.enabled)
        ('Account', const _BuilderlabAccountSection()),
    ];
    Widget detail(String name, Widget section) => Builder(
      builder: (detailContext) => FrostedScaffold(
        useUtilitySurfaceTheme: true,
        appBar: FrostedAppBar(
          nativeTitle: name,
          title: Text(name),
          automaticallyImplyLeading: !usesConversationPanes(
            detailContext,
            windowSize:
                View.of(detailContext).physicalSize /
                View.of(detailContext).devicePixelRatio,
          ),
        ),
        body: ListView(
          padding: EdgeInsets.only(
            top: frostedAppBarHeight(detailContext),
            bottom: MediaQuery.paddingOf(detailContext).bottom + Grid.xs,
          ),
          children: [section],
        ),
      ),
    );
    final controller = useMemoized(ConversationPaneController.new);
    useEffect(() => controller.dispose, [controller]);
    useEffect(
      () {
        if (wide &&
            (controller.selectedId == null ||
                (controller.selectedId == 'Notifications' &&
                    !sections.any(
                      (section) => section.$1 == 'Notifications',
                    )))) {
          final section = sections.first;
          controller.open(
            section.$1,
            (_) => detail(section.$1, section.$2),
            animate: false,
          );
        }
        return null;
      },
      [
        wide,
        controller.selectedId,
        sections.map((section) => section.$1).join(','),
      ],
    );
    useListenable(controller);
    // Keep feedback local even while Home is mounted during the transition.
    return Theme(
      data: utilitySurfaceThemeData(Theme.of(context)),
      child: AdaptiveConversationDestination(
        preferredListWidth: avatarFirst ? 375 : null,
        emptyLabel: 'Select a setting',
        listToolbarSlots: 1,
        controller: controller,
        persistentSelection: sections.any(
          (section) => section.$1 == controller.selectedId,
        ),
        child: Builder(
          builder: (paneContext) => ScaffoldMessenger(
            child: FrostedScaffold(
              useUtilitySurfaceTheme: true,
              appBar: FrostedAppBar(
                nativeTitle: avatarFirst ? '' : 'Settings',
                railOnly: avatarFirst,
                nativeLeading: IosNavigationAction(
                  label: 'Close settings',
                  symbol: 'xmark',
                  onPressed: () => Navigator.of(context).pop(),
                ),
                automaticallyImplyLeading: false,
                horizontalInset: Grid.gutter,
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
              body: ListView(
                padding: EdgeInsets.only(
                  top: topSectionHeight,
                  bottom: MediaQuery.paddingOf(context).bottom + Grid.xs,
                ),
                children: [
                  profileHeader,
                  if (wide)
                    AppListCard(
                      dividerIndent: Grid.xs,
                      verticalPadding: Grid.twelve,
                      children: [
                        for (final section in sections)
                          AppListRow(
                            title: section.$1,
                            trailing: const _RowChevron(),
                            onTap: () => controller.open(
                              section.$1,
                              (_) => detail(section.$1, section.$2),
                              animate: false,
                            ),
                          ),
                      ],
                    )
                  else ...[
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
                  if (!wide && HostedEnvironment.enabled)
                    const _BuilderlabAccountSection(),
                  // An internal build's only way out of its organization, rarely
                  // needed, so it sits last.
                  if (OrganizationEnvironment.enabled)
                    AppListCard(
                      verticalPadding: Grid.twelve,
                      children: [
                        AppListRow(
                          key: const Key('settings-sign-out'),
                          icon: BuzzIcons.logOut,
                          title: 'Sign out',
                          titleColor: context.colors.error,
                          onTap: () => confirmOrganizationSignOut(context, ref),
                        ),
                      ],
                    ),
                  if (packageInfo.hasData)
                    _VersionFooter(
                      version: packageInfo.data!.version,
                      buildNumber: packageInfo.data!.buildNumber,
                    ),
                ],
              ),
            ),
          ),
        ),
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
    final label = buildNumber.isEmpty
        ? 'v$version'
        : 'v$version ($buildNumber)';
    return SafeArea(
      top: false,
      child: Center(
        child: Semantics(
          button: true,
          onTapHint: 'Copy version',
          child: InkWell(
            borderRadius: BorderRadius.circular(Grid.xs),
            onTap: () =>
                copyToClipboard(context, label, message: 'Version copied'),
            // Keep the text where it was and grow the tap target downward
            // to the platform minimum.
            child: ConstrainedBox(
              constraints: const BoxConstraints(
                minHeight: kMinInteractiveDimension,
              ),
              child: Padding(
                padding: const EdgeInsets.fromLTRB(
                  Grid.xs,
                  Grid.xxs,
                  Grid.xs,
                  Grid.xs,
                ),
                child: Text(
                  label,
                  style: context.textTheme.bodySmall?.copyWith(
                    color: context.colors.onSurfaceVariant.withValues(
                      alpha: 0.6,
                    ),
                  ),
                ),
              ),
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
