import 'package:buzz/shared/widgets/feedback_snackbar.dart';
import 'community_link_focus.dart';
import '../../shared/widgets/ios_community_link_input.dart';
import 'dart:async';
import '../../shared/hosted/hosted_flow_provider.dart';
import '../../shared/hosted/hosted_flow.dart';
import '../../shared/hosted/hosted_community_creation.dart';
import '../../shared/hosted/hosted_environment.dart';
import '../../shared/hosted/hosted_flow_problem_message.dart';
import 'dart:math' show sqrt2;
import 'dart:math' as math;
import 'dart:ui' show ImageFilter;

import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import 'package:flutter/services.dart';
import 'package:flutter/semantics.dart';
import 'package:flutter/gestures.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:flutter_svg/flutter_svg.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';

import '../../shared/security/sensitive_action_authorizer.dart';
import '../../shared/error_haptic.dart';
import '../../shared/deeplink/deep_link.dart';
import '../../shared/deeplink/community_link_input.dart';
import '../../shared/deeplink/pending_deep_link_provider.dart';
import '../../shared/community/community.dart';
import '../../shared/community/community_loading_surface.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/ios_glass_navigation_button.dart';
import 'pairing_page/onboarding_wordmark.dart';
import 'pairing_provider.dart';
import 'add_community_page.dart';
import '../../shared/widgets/onboarding_step_page.dart';
import 'pairing_qr_scanner.dart';

part 'pairing_page/onboarding_background.dart';
part 'pairing_page/website_welcome_background.dart';
part 'pairing_page/website_bee_swarm.dart';
part 'pairing_page/website_bee_motion.dart';
part 'pairing_page/onboarding_colors.dart';
part 'pairing_page/onboarding_glass_button.dart';
part 'pairing_page/pairing_welcome_view.dart';
part 'pairing_page/two_finger_double_tap.dart';
part 'pairing_page/community_link_form.dart';
part 'pairing_page/onboarding_connection_view.dart';
part 'pairing_page/sas_verification_view.dart';
part 'pairing_page/pairing_code_entry.dart';
part 'pairing_page/pairing_error_shake.dart';

class PairingPage extends HookConsumerWidget {
  /// When true, the pairing page is being used to add a new community
  /// (user is already authenticated with at least one community).
  final bool addingCommunity;
  final bool identityRecoveryOnly;
  final bool signedInOnboarding;
  final bool hasKnownCommunities;
  final VoidCallback? onAlreadyUseBuzz;
  final VoidCallback? onWelcomeBack;
  final bool connectionLanding;
  final bool showConnectionOptions;
  final bool existingUser;
  final bool enteringCommunityLink;
  final bool creatingCommunity;
  final ValueChanged<String>? onSubmitCommunity;
  final VoidCallback? onCommunityLink;
  final VoidCallback? onCreateCommunity;
  final VoidCallback? onBuilderlab;
  final String? accountStatus;
  final Widget? setupContent;
  final String builderlabLabel;
  final bool builderlabBusy;
  final bool signInPending;

  const PairingPage({
    super.key,
    this.addingCommunity = false,
    this.identityRecoveryOnly = false,
    this.signedInOnboarding = false,
    this.hasKnownCommunities = false,
    this.onAlreadyUseBuzz,
    this.onWelcomeBack,
    this.connectionLanding = false,
    this.showConnectionOptions = true,
    this.existingUser = false,
    this.enteringCommunityLink = false,
    this.creatingCommunity = false,
    this.onSubmitCommunity,
    this.onCommunityLink,
    this.onCreateCommunity,
    this.onBuilderlab,
    this.accountStatus,
    this.setupContent,
    this.builderlabLabel = 'Connect to Builderlab',
    this.builderlabBusy = false,
    this.signInPending = false,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final hostedFlow = useListenable(
      HostedEnvironment.enabled ? ref.watch(hostedFlowProvider) : null,
    );
    final pairingState = ref.watch(pairingProvider);
    final enrolledBiometrics = ref.watch(enrolledBiometricsProvider);
    final codeController = useTextEditingController();
    final fallbackScannerVisible = useState(false);
    final islandScannerProgress = useValueNotifier(0.0);
    final pairingCodeExpanded = useState(false);
    final optionsExpanded = useState(false);
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
        if (connectionLanding && !existingUser) {
          final uri = Uri.tryParse(code.trim());
          final link = uri == null ? null : parseBuzzDeepLink(uri);
          if (link is InviteDeepLink || link is AddCommunityDeepLink) {
            ref.read(pendingDeepLinkProvider.notifier).open(uri!);
            return;
          }
        }
        if (identityRecoveryOnly &&
            Uri.tryParse(code)?.queryParameters['mode'] != 'recover') {
          ScaffoldMessenger.of(context).showFeedbackSnackBar(
            const SnackBar(content: Text('Scan a desktop recovery code.')),
            kind: ToastKind.generic,
          );
          return;
        }
        await ref.read(pairingProvider.notifier).pair(code);
      }
    }

    Future<void> openScanner() async {
      unawaited(HapticFeedback.selectionClick());
      final usesDynamicIslandPortal = await usesDynamicIslandQrScannerPortal();
      if (!context.mounted) {
        return;
      }

      if (!usesDynamicIslandPortal) {
        fallbackScannerVisible.value = true;
        return;
      }

      String? code;
      try {
        code = await showDynamicIslandPairingQrScanner(
          context,
          onProgress: (value) {
            if (context.mounted) islandScannerProgress.value = value;
          },
        );
      } finally {
        if (context.mounted) islandScannerProgress.value = 0;
      }
      await handleScannerResult(code);
    }

    final isVerifyingSas = pairingState.status == PairingStatus.confirmingSas;
    final websiteWelcome =
        connectionLanding &&
        !addingCommunity &&
        !existingUser &&
        !enteringCommunityLink &&
        !creatingCommunity &&
        setupContent == null &&
        !isVerifyingSas;
    final onboardingSystemOverlayStyle =
        (context._onboardingIsDark && !websiteWelcome
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

                        iconColor: context._onboardingInk,

                        title: identityRecoveryOnly
                            ? const Text('Send to Desktop')
                            : null,
                      ),
                    ],
                  ),
                )
              : PreferredSize(
                  preferredSize: Size.fromHeight(
                    frostedAppBarHeight(context) -
                        MediaQuery.paddingOf(context).top,
                  ),
                  child: Stack(
                    children: [
                      FrostedAppBar(
                        alwaysFrosted: true,
                        leading: IconButton(
                          tooltip: 'Back',
                          icon: const Icon(BuzzIcons.arrowLeft),
                          onPressed: () => Navigator.of(context).maybePop(),
                        ),
                        iconColor: context._onboardingInk,
                        title: identityRecoveryOnly
                            ? const Text('Send to Desktop')
                            : null,
                      ),
                    ],
                  ),
                )
        : null;

    final showLoading =
        pairingState.status == PairingStatus.transferring ||
        pairingState.status == PairingStatus.storing ||
        pairingState.status == PairingStatus.success;
    final destinationRelay = pairingState.destinationRelayUrl;
    final verificationView = _SasVerificationView(
      key: ValueKey(pairingState.sasCode),
      fullScreen: addingCommunity && connectionLanding,
      sasCode: pairingState.sasCode ?? '------',
      verifyDesktopCode: pairingState.requiresDesktopCode
          ? ref.read(pairingProvider.notifier).verifyDesktopCode
          : null,
      confirmed: pairingState.userConfirmedSas,
      sendsIdentityToDesktop: pairingState.sendsIdentityToDesktop,
      biometricLabel: biometricProtectionLabel(
        defaultTargetPlatform,
        enrolledBiometrics.value ?? const [],
      ),
      errorMessage: pairingState.errorMessage,
      onProtectionChanged: (value) =>
          ref.read(pairingProvider.notifier).setProtectSensitiveActions(value),
      onConfirm: () => ref.read(pairingProvider.notifier).confirmSas(),
      onDeny: () => ref.read(pairingProvider.notifier).denySas(),
    );
    final pairingScaffold = showLoading
        ? CommunityLoadingSurface(
            key: const Key('pairing-community-loading'),
            name: destinationRelay == null
                ? null
                : Community.nameFromUrl(destinationRelay),
            relayUrl: destinationRelay,
          )
        : isVerifyingSas
        ? addingCommunity && connectionLanding
              ? verificationView
              : AnnotatedRegion<SystemUiOverlayStyle>(
                  key: const Key('pairing-sas-system-overlay'),
                  value: onboardingSystemOverlayStyle,
                  child: _OnboardingBackground(
                    child: Scaffold(
                      resizeToAvoidBottomInset:
                          hostedFlow?.stage != HostedFlowStage.profile,
                      backgroundColor: Colors.transparent,
                      body: SafeArea(
                        child: Padding(
                          padding: const EdgeInsets.symmetric(
                            horizontal: Grid.sm,
                          ),
                          child: verificationView,
                        ),
                      ),
                    ),
                  ),
                )
        : (addingCommunity || (signedInOnboarding && !creatingCommunity)) &&
              connectionLanding
        ? ValueListenableBuilder<double>(
            valueListenable: islandScannerProgress,
            builder: (context, progress, _) => AddCommunityPage(
              scannerProgress: progress,
              onboarding: signedInOnboarding,
              onFindCommunities: signedInOnboarding && hasKnownCommunities
                  ? onBuilderlab
                  : null,
              onCreateCommunity: signedInOnboarding ? onCreateCommunity : null,
              enteringCommunityLink: enteringCommunityLink,
              existingUser: existingUser,
              onBack: onWelcomeBack ?? () => Navigator.of(context).pop(),
              onCommunityLink: onCommunityLink,
              onPairing: onAlreadyUseBuzz,
              onScan: openScanner,
              onOpenCommunity: (uri) {
                ref.read(hostedInlineEntryProvider.notifier).setActive(true);
                ref.read(pendingDeepLinkProvider.notifier).open(uri);
              },
              onSubmitPairing: (code) => unawaited(handleScannerResult(code)),
              accountStatus: signedInOnboarding ? null : accountStatus,
              busy:
                  isBusy ||
                  builderlabBusy ||
                  (hostedFlow != null &&
                      !hostedFlow.accountOnly &&
                      (hostedFlow.stage == HostedFlowStage.working ||
                          hostedFlow.stage == HostedFlowStage.signIn)),
              canSubmit:
                  hostedFlow == null ||
                  hostedFlow.accountOnly ||
                  hostedFlow.problem == null ||
                  hostedFlow.canRetry,
              errorMessage:
                  pairingState.errorMessage ??
                  (hostedFlow != null &&
                          !hostedFlow.accountOnly &&
                          hostedFlow.problem != null
                      ? hostedFlowProblemMessage(hostedFlow.problem)
                      : null),
              onInputChanged: () {
                if (hostedFlow != null &&
                    !hostedFlow.accountOnly &&
                    hostedFlow.problem != null) {
                  unawaited(hostedFlow.leave());
                }
              },
            ),
          )
        : AnnotatedRegion<SystemUiOverlayStyle>(
            key: const Key('pairing-onboarding-system-overlay'),
            value: onboardingSystemOverlayStyle,
            child: _OnboardingBackground(
              websiteWelcome: websiteWelcome,
              scannerProgress: islandScannerProgress,
              child: Scaffold(
                resizeToAvoidBottomInset:
                    hostedFlow?.stage != HostedFlowStage.profile,
                backgroundColor: Colors.transparent,
                appBar: pairingAppBar,
                body: SafeArea(
                  child: ValueListenableBuilder<double>(
                    valueListenable: islandScannerProgress,
                    builder: (context, progress, child) => ExcludeFocus(
                      excluding: progress > 0,
                      child: ExcludeSemantics(
                        excluding: progress > 0,
                        child: IgnorePointer(
                          ignoring: progress > 0,
                          child: Opacity(
                            key: const Key('scanner-welcome-opacity'),
                            opacity: 1 - progress,
                            child: Transform.scale(
                              key: const Key('scanner-welcome-scale'),
                              scale: MediaQuery.disableAnimationsOf(context)
                                  ? 1
                                  : 1 - 0.04 * progress,
                              child: child,
                            ),
                          ),
                        ),
                      ),
                    ),
                    child: _PairingWelcomeView(
                      optionsExpanded: optionsExpanded,
                      onAlreadyUseBuzz: onAlreadyUseBuzz,
                      onWelcomeBack: onWelcomeBack,
                      connectionLanding: connectionLanding,
                      showConnectionOptions: showConnectionOptions,
                      existingUser: existingUser,
                      enteringCommunityLink: enteringCommunityLink,
                      creatingCommunity: creatingCommunity,
                      onSubmitCommunity: onSubmitCommunity,
                      onCommunityLink: onCommunityLink,
                      onCreateCommunity: onCreateCommunity,
                      onOpenCommunity: (uri) {
                        FocusManager.instance.primaryFocus?.unfocus();
                        ref.read(pendingDeepLinkProvider.notifier).open(uri);
                      },
                      onBuilderlab: onBuilderlab,
                      accountStatus: signedInOnboarding ? null : accountStatus,
                      setupContent: setupContent,
                      builderlabLabel: builderlabLabel,
                      builderlabBusy: builderlabBusy,
                      signInPending: signInPending,
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
