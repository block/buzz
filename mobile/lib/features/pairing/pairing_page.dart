import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

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
    final themedSystemOverlayStyle =
        (context.theme.brightness == Brightness.dark
                ? SystemUiOverlayStyle.light
                : SystemUiOverlayStyle.dark)
            .copyWith(statusBarColor: Colors.transparent);
    final pairingAppBar = addingCommunity
        ? AppBar(
            foregroundColor: isVerifyingSas
                ? context.colors.onSurface
                : _onboardingInk,
            systemOverlayStyle: isVerifyingSas
                ? themedSystemOverlayStyle
                : SystemUiOverlayStyle.dark.copyWith(
                    statusBarColor: Colors.transparent,
                  ),
            leading: IconButton(
              icon: const Icon(LucideIcons.arrowLeft),
              onPressed: () => Navigator.of(context).pop(),
            ),
            title: Text(
              identityRecoveryOnly ? 'Send to Desktop' : 'Add Community',
              style: isVerifyingSas
                  ? null
                  : context.textTheme.titleMedium?.copyWith(
                      color: _onboardingInk,
                    ),
            ),
          )
        : null;

    final pairingScaffold = isVerifyingSas
        ? AnnotatedRegion<SystemUiOverlayStyle>(
            key: const Key('pairing-sas-system-overlay'),
            value: themedSystemOverlayStyle,
            child: Scaffold(
              backgroundColor: context.colors.surface,
              appBar: pairingAppBar,
              body: SafeArea(
                child: Padding(
                  padding: const EdgeInsets.symmetric(horizontal: Grid.sm),
                  child: _SasVerificationView(
                    sasCode: pairingState.sasCode ?? '------',
                    confirmed: pairingState.userConfirmedSas,
                    sendsIdentityToDesktop: pairingState.sendsIdentityToDesktop,
                    onConfirm: () =>
                        ref.read(pairingProvider.notifier).confirmSas(),
                    onDeny: () => ref.read(pairingProvider.notifier).denySas(),
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
  final String sasCode;
  final bool confirmed;
  final bool sendsIdentityToDesktop;
  final VoidCallback onConfirm;
  final VoidCallback onDeny;

  const _SasVerificationView({
    required this.sasCode,
    required this.confirmed,
    required this.sendsIdentityToDesktop,
    required this.onConfirm,
    required this.onDeny,
  });

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisAlignment: MainAxisAlignment.center,
      children: [
        const Spacer(flex: 2),

        Icon(LucideIcons.shieldCheck, size: 56, color: context.colors.primary),
        const SizedBox(height: Grid.sm),

        Text('Verify Security Code', style: context.textTheme.headlineSmall),
        const SizedBox(height: Grid.xs),

        Text(
          confirmed
              ? 'Waiting for desktop to confirm...'
              : 'Does your desktop app show this code?',
          textAlign: TextAlign.center,
          style: context.textTheme.bodyMedium?.copyWith(
            color: context.colors.onSurfaceVariant,
          ),
        ),

        const SizedBox(height: Grid.lg),

        // Large SAS code display
        Container(
          padding: const EdgeInsets.symmetric(horizontal: 32, vertical: 20),
          decoration: BoxDecoration(
            color: context.colors.primaryContainer.withValues(alpha: 0.3),
            borderRadius: BorderRadius.circular(16),
            border: Border.all(
              color: context.colors.primary.withValues(alpha: 0.3),
              width: 2,
            ),
          ),
          child: Text(
            '${sasCode.substring(0, 3)} ${sasCode.substring(3)}',
            style: context.textTheme.displayMedium?.copyWith(
              fontFamily: 'GeistMono',
              fontWeight: FontWeight.w700,
              letterSpacing: 8,
              color: context.colors.primary,
            ),
          ),
        ),

        const SizedBox(height: Grid.lg),

        Text(
          sendsIdentityToDesktop
              ? 'This sends your full Buzz identity to the desktop\nand grants it permanent access. Only confirm a\ndesktop you trust and a recovery you started.'
              : 'You are about to transfer your Buzz identity\nto this device. Only confirm if you initiated\nthis pairing from your desktop.',
          textAlign: TextAlign.center,
          style: context.textTheme.bodySmall?.copyWith(
            color: context.colors.onSurfaceVariant,
          ),
        ),

        const SizedBox(height: Grid.lg),

        // Confirm / Deny buttons
        if (confirmed)
          Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              BuzzLoadingIndicator(
                size: 24,
                color: context.colors.primary,
                semanticLabel: 'Connecting',
              ),
              const SizedBox(width: Grid.twelve),
              Text(
                'Confirmed — waiting for desktop',
                style: context.textTheme.bodySmall?.copyWith(
                  color: context.colors.onSurfaceVariant,
                ),
              ),
            ],
          )
        else
          Row(
            mainAxisAlignment: MainAxisAlignment.center,
            children: [
              Expanded(
                child: OutlinedButton.icon(
                  onPressed: onDeny,
                  icon: const Icon(LucideIcons.x),
                  label: const Text('Cancel'),
                ),
              ),
              const SizedBox(width: Grid.sm),
              Expanded(
                child: FilledButton.icon(
                  onPressed: onConfirm,
                  icon: const Icon(LucideIcons.check),
                  label: const Text('Codes Match'),
                ),
              ),
            ],
          ),

        const Spacer(flex: 3),
      ],
    );
  }
}
