import 'dart:async';
import 'dart:math' show sqrt2;
import 'dart:ui' show ImageFilter;

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/security/sensitive_action_authorizer.dart';
import '../../shared/error_haptic.dart';
import '../../shared/community/community.dart';
import '../../shared/community/community_loading_surface.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import 'pairing_page/onboarding_wordmark.dart';
import 'pairing_provider.dart';
import 'pairing_qr_scanner.dart';

part 'pairing_page/onboarding_background.dart';
part 'pairing_page/onboarding_colors.dart';
part 'pairing_page/onboarding_glass_button.dart';
part 'pairing_page/pairing_welcome_view.dart';
part 'pairing_page/sas_verification_view.dart';
part 'pairing_page/pairing_code_entry.dart';
part 'pairing_page/pairing_error_shake.dart';

class PairingPage extends HookConsumerWidget {
  /// When true, the pairing page is being used to add a new community
  /// (user is already authenticated with at least one community).
  final bool addingCommunity;
  final bool identityRecoveryOnly;

  const PairingPage({
    super.key,
    this.addingCommunity = false,
    this.identityRecoveryOnly = false,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final pairingState = ref.watch(pairingProvider);
    final enrolledBiometrics = ref.watch(enrolledBiometricsProvider);
    final codeController = useTextEditingController();
    final fallbackScannerVisible = useState(false);
    final pairingCodeExpanded = useState(false);
    final isBusy =
        pairingState.status == PairingStatus.connecting ||
        pairingState.status == PairingStatus.transferring ||
        pairingState.status == PairingStatus.storing;

    // When adding a community and pairing succeeds, pop back.
    if (addingCommunity && pairingState.status == PairingStatus.success) {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (context.mounted) {
          final route = ModalRoute.of(context);
          if (route != null && route.isActive) {
            final notifier = ref.read(pairingProvider.notifier);
            Navigator.of(context).removeRoute(route);
            // Reset only after removal so the dismissed page cannot flash its
            // scanner, and subsequent recovery/onboarding starts fresh.
            notifier.reset();
          }
        }
      });
    }

    Future<void> handleScannerResult(String? code) async {
      if (code != null && context.mounted) {
        if (identityRecoveryOnly &&
            Uri.tryParse(code)?.queryParameters['mode'] != 'recover') {
          ScaffoldMessenger.of(context).showSnackBar(
            const SnackBar(content: Text('Scan a desktop recovery code.')),
          );
          return;
        }
        await ref.read(pairingProvider.notifier).pair(code);
      }
    }

    Future<void> openScanner() async {
      final usesDynamicIslandPortal = await usesDynamicIslandQrScannerPortal();
      if (!context.mounted) {
        return;
      }

      if (!usesDynamicIslandPortal) {
        fallbackScannerVisible.value = true;
        return;
      }

      final code = await showDynamicIslandPairingQrScanner(context);
      await handleScannerResult(code);
    }

    final isVerifyingSas = pairingState.status == PairingStatus.confirmingSas;
    final onboardingSystemOverlayStyle =
        (context._onboardingIsDark
                ? SystemUiOverlayStyle.light
                : SystemUiOverlayStyle.dark)
            .copyWith(statusBarColor: Colors.transparent);
    final pairingAppBar = addingCommunity
        ? defaultTargetPlatform == TargetPlatform.iOS
              ? PreferredSize(
                  preferredSize: const Size.fromHeight(44),
                  child: Stack(
                    children: [
                      FrostedAppBar(
                        nativeTitle: identityRecoveryOnly
                            ? 'Send to Desktop'
                            : '',
                        frosted: false,
                        showBottomDivider: false,
                        iconColor: context._onboardingInk,

                        title: identityRecoveryOnly
                            ? const Text('Send to Desktop')
                            : null,
                      ),
                    ],
                  ),
                )
              : AppBar(
                  backgroundColor: Colors.transparent,
                  surfaceTintColor: Colors.transparent,
                  elevation: 0,
                  scrolledUnderElevation: 0,
                  foregroundColor: context._onboardingInk,
                  systemOverlayStyle: onboardingSystemOverlayStyle,
                  leadingWidth: Theme.of(context).platform == TargetPlatform.iOS
                      ? Grid.quarter + iosGlassChannelHeaderLeadingWidth
                      : null,
                  leading: Theme.of(context).platform == TargetPlatform.iOS
                      ? Padding(
                          padding: const EdgeInsets.only(left: Grid.quarter),
                          child: IosGlassNavigationButton(
                            key: const ValueKey('pairing-ios-glass-back'),
                            icon: IosGlassNavigationIcon.back,
                            semanticLabel: 'Back',
                            onPressed: () => Navigator.of(context).maybePop(),
                            width: iosGlassChannelHeaderLeadingWidth,
                            buttonCenterX: iosGlassChannelHeaderButtonCenterX,
                            foregroundColor: context._onboardingInk,
                          ),
                        )
                      : IconButton(
                          icon: const Icon(LucideIcons.arrowLeft),
                          tooltip: 'Back',
                          onPressed: () => Navigator.of(context).pop(),
                        ),
                  title: identityRecoveryOnly
                      ? Text(
                          'Send to Desktop',
                          style: context.textTheme.titleMedium?.copyWith(
                            color: context._onboardingInk,
                          ),
                        )
                      : null,
                )
        : null;

    final showLoading =
        pairingState.status == PairingStatus.transferring ||
        pairingState.status == PairingStatus.storing ||
        pairingState.status == PairingStatus.success;
    final destinationRelay = pairingState.destinationRelayUrl;
    final pairingScaffold = showLoading
        ? CommunityLoadingSurface(
            key: const Key('pairing-community-loading'),
            name: destinationRelay == null
                ? null
                : Community.nameFromUrl(destinationRelay),
            relayUrl: destinationRelay,
          )
        : isVerifyingSas
        ? AnnotatedRegion<SystemUiOverlayStyle>(
            key: const Key('pairing-sas-system-overlay'),
            value: onboardingSystemOverlayStyle,
            child: _OnboardingBackground(
              child: Scaffold(
                backgroundColor: Colors.transparent,
                body: SafeArea(
                  child: Padding(
                    padding: const EdgeInsets.symmetric(horizontal: Grid.sm),
                    child: _SasVerificationView(
                      key: ValueKey(pairingState.sasCode),
                      sasCode: pairingState.sasCode ?? '------',
                      verifyDesktopCode: pairingState.requiresDesktopCode
                          ? ref.read(pairingProvider.notifier).verifyDesktopCode
                          : null,
                      confirmed: pairingState.userConfirmedSas,
                      sendsIdentityToDesktop:
                          pairingState.sendsIdentityToDesktop,
                      biometricLabel: biometricProtectionLabel(
                        defaultTargetPlatform,
                        enrolledBiometrics.value ?? const [],
                      ),
                      errorMessage: pairingState.errorMessage,
                      onProtectionChanged: (value) => ref
                          .read(pairingProvider.notifier)
                          .setProtectSensitiveActions(value),
                      onConfirm: () =>
                          ref.read(pairingProvider.notifier).confirmSas(),
                      onDeny: () =>
                          ref.read(pairingProvider.notifier).denySas(),
                    ),
                  ),
                ),
              ),
            ),
          )
        : AnnotatedRegion<SystemUiOverlayStyle>(
            key: const Key('pairing-onboarding-system-overlay'),
            value: onboardingSystemOverlayStyle,
            child: _OnboardingBackground(
              child: Scaffold(
                backgroundColor: Colors.transparent,
                appBar: pairingAppBar,
                body: SafeArea(
                  child: _PairingWelcomeView(
                    codeController: codeController,
                    isBusy: isBusy,
                    pairingCodeExpanded: pairingCodeExpanded.value,
                    errorMessage: pairingState.status == PairingStatus.error
                        ? pairingState.errorMessage
                        : null,
                    onScan: openScanner,
                    onTogglePairingCode: () {
                      pairingCodeExpanded.value = !pairingCodeExpanded.value;
                    },
                    onConnect: () {
                      final code = codeController.text.trim();
                      if (code.isNotEmpty) {
                        unawaited(handleScannerResult(code));
                      }
                    },
                  ),
                ),
              ),
            ),
          );

    final appSurface = PopScope(
      key: const Key('pairing-pop-scope'),
      onPopInvokedWithResult: (didPop, _) {
        if (didPop && pairingState.status != PairingStatus.success) {
          ref.read(pairingProvider.notifier).reset();
        }
      },
      child: pairingScaffold,
    );

    if (!fallbackScannerVisible.value) {
      return appSurface;
    }

    return FallbackPairingQrScanner(
      appSurface: appSurface,
      onClosed: (code) {
        fallbackScannerVisible.value = false;
        unawaited(handleScannerResult(code));
      },
    );
  }
}
