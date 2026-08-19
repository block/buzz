import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/security/sensitive_action_authorizer.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_loading_indicator.dart';
import '../../shared/widgets/tappable_flapping_bee.dart';
import 'pairing_provider.dart';
import 'pairing_qr_scanner.dart';

part 'pairing_page/onboarding_background.dart';
part 'pairing_page/pairing_welcome_view.dart';

const _onboardingChartreuse = Color(0xFFD7D72E);
const _onboardingShellBottom = Color(0xFFD7E7F6);
const _onboardingCtaLabel = Color(0xFFD7E6F0);
const _onboardingInk = Color(0xFF111111);
const _onboardingMutedInk = Color(0xB3111111);
const _onboardingErrorInk = Color(0xFF7A1025);

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
          ref.read(pairingProvider.notifier).reset();
          Navigator.of(context).pop();
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
    final onboardingSystemOverlayStyle = SystemUiOverlayStyle.dark.copyWith(
      statusBarColor: Colors.transparent,
    );
    final pairingAppBar = addingCommunity
        ? AppBar(
            foregroundColor: _onboardingInk,
            systemOverlayStyle: onboardingSystemOverlayStyle,
            leading: IconButton(
              icon: const Icon(LucideIcons.arrowLeft),
              onPressed: () => Navigator.of(context).pop(),
            ),
            title: Text(
              identityRecoveryOnly ? 'Send to Desktop' : 'Add Community',
              style: context.textTheme.titleMedium?.copyWith(
                color: _onboardingInk,
              ),
            ),
          )
        : null;

    final pairingScaffold = isVerifyingSas
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
                      sasCode: pairingState.sasCode ?? '------',
                      confirmed: pairingState.userConfirmedSas,
                      sendsIdentityToDesktop:
                          pairingState.sendsIdentityToDesktop,
                      protectSensitiveActions:
                          pairingState.protectSensitiveActions,
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
            value: SystemUiOverlayStyle.dark.copyWith(
              statusBarColor: Colors.transparent,
            ),
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
        if (didPop) {
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

/// SAS verification screen shown during NIP-AB pairing.
class _SasVerificationView extends StatelessWidget {
  static const _digitSize = 54.0;
  static const _digitGap = 6.0;
  static const _digitGroupGap = 14.0;

  final String sasCode;
  final bool confirmed;
  final bool sendsIdentityToDesktop;
  final bool protectSensitiveActions;
  final String biometricLabel;
  final String? errorMessage;
  final ValueChanged<bool> onProtectionChanged;
  final VoidCallback onConfirm;
  final VoidCallback onDeny;

  const _SasVerificationView({
    required this.sasCode,
    required this.confirmed,
    required this.sendsIdentityToDesktop,
    required this.protectSensitiveActions,
    required this.biometricLabel,
    required this.errorMessage,
    required this.onProtectionChanged,
    required this.onConfirm,
    required this.onDeny,
  });

  @override
  Widget build(BuildContext context) {
    final verificationContent = Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Text(
          'Confirm desktop code',
          textAlign: TextAlign.center,
          style: context.textTheme.headlineSmall?.copyWith(
            color: _onboardingInk,
            fontWeight: FontWeight.w600,
            letterSpacing: -0.4,
          ),
        ),
        const SizedBox(height: Grid.xxs),
        Text(
          sendsIdentityToDesktop
              ? 'Make sure the six-digit code matches on both devices. Your full Buzz identity will transfer to the desktop and grant it permanent access. Only continue if you started this recovery.'
              : 'Make sure the six-digit code matches on both devices. Your Buzz identity will transfer to this device. Only continue if you started this pairing from your desktop.',
          textAlign: TextAlign.center,
          style: context.textTheme.bodyMedium?.copyWith(
            color: _onboardingMutedInk,
          ),
        ),
        const SizedBox(height: Grid.md),
        Semantics(
          label:
              'Confirmation code ${sasCode.substring(0, 3)} ${sasCode.substring(3)}',
          child: ExcludeSemantics(
            child: FittedBox(
              fit: BoxFit.scaleDown,
              child: Row(
                mainAxisSize: MainAxisSize.min,
                children: [
                  for (var index = 0; index < sasCode.length; index++) ...[
                    if (index > 0)
                      SizedBox(width: index == 3 ? _digitGroupGap : _digitGap),
                    Container(
                      key: Key('pairing-sas-code-digit-${index + 1}'),
                      width: _digitSize,
                      padding: const EdgeInsets.symmetric(vertical: Grid.xs),
                      alignment: Alignment.center,
                      decoration: BoxDecoration(
                        color: Colors.white.withValues(alpha: 0.7),
                        borderRadius: BorderRadius.circular(12),
                        border: Border.all(
                          color: context.colors.primary.withValues(alpha: 0.15),
                        ),
                      ),
                      child: Text(
                        sasCode[index],
                        style: context.textTheme.displaySmall?.copyWith(
                          fontWeight: FontWeight.w600,
                          color: _onboardingInk,
                        ),
                      ),
                    ),
                  ],
                ],
              ),
            ),
          ),
        ),
        const SizedBox(height: Grid.sm),
        if (!sendsIdentityToDesktop)
          CheckboxListTile(
            key: const Key('protect-sensitive-actions-checkbox'),
            value: protectSensitiveActions,
            onChanged: confirmed
                ? null
                : (value) => onProtectionChanged(value ?? false),
            activeColor: _onboardingInk,
            checkColor: _onboardingCtaLabel,
            side: const BorderSide(color: _onboardingInk),
            controlAffinity: ListTileControlAffinity.leading,
            contentPadding: EdgeInsets.zero,
            title: Text(
              biometricLabel,
              style: context.textTheme.bodyMedium?.copyWith(
                color: _onboardingInk,
                fontWeight: FontWeight.w600,
              ),
            ),
            subtitle: Text(
              'For secure actions',
              style: context.textTheme.bodySmall?.copyWith(
                color: _onboardingMutedInk,
              ),
            ),
          ),
        if (errorMessage != null) ...[
          const SizedBox(height: Grid.xs),
          Text(
            errorMessage!,
            textAlign: TextAlign.center,
            style: context.textTheme.bodySmall?.copyWith(
              color: _onboardingErrorInk,
            ),
          ),
        ],
      ],
    );

    final verificationActions = confirmed
        ? Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              BuzzLoadingIndicator(
                size: 24,
                color: _onboardingInk,
                semanticLabel: 'Connecting',
              ),
              const SizedBox(width: Grid.twelve),
              Text(
                'Confirmed — waiting for desktop',
                style: context.textTheme.bodySmall?.copyWith(
                  color: _onboardingMutedInk,
                ),
              ),
            ],
          )
        : SizedBox(
            width: double.infinity,
            child: Column(
              crossAxisAlignment: CrossAxisAlignment.stretch,
              children: [
                FilledButton.icon(
                  style: _onboardingButtonStyle,
                  onPressed: onConfirm,
                  icon: const Icon(LucideIcons.check),
                  label: const Text('Codes match'),
                ),
                const SizedBox(height: Grid.xxs),
                TextButton(
                  style: _onboardingSecondaryButtonStyle.copyWith(
                    minimumSize: const WidgetStatePropertyAll(
                      Size.fromHeight(48),
                    ),
                  ),
                  onPressed: onDeny,
                  child: const Text('Cancel'),
                ),
              ],
            ),
          );

    return Column(
      children: [
        Expanded(
          child: LayoutBuilder(
            builder: (context, constraints) {
              final verticalPadding = Grid.sm * 2;
              final minimumContentHeight =
                  constraints.maxHeight > verticalPadding
                  ? constraints.maxHeight - verticalPadding
                  : 0.0;
              return SingleChildScrollView(
                padding: const EdgeInsets.symmetric(vertical: Grid.sm),
                child: ConstrainedBox(
                  constraints: BoxConstraints(minHeight: minimumContentHeight),
                  child: Center(child: verificationContent),
                ),
              );
            },
          ),
        ),
        verificationActions,
        const SizedBox(height: Grid.sm),
      ],
    );
  }
}
