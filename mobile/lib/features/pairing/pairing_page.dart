import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/tappable_flapping_bee.dart';
import 'pairing_provider.dart';
import 'pairing_qr_scanner.dart';

const _onboardingChartreuse = Color(0xFFD7D72E);
const _onboardingShellBottom = Color(0xFFD7E7F6);
const _onboardingCtaLabel = Color(0xFFD7E6F0);
const _onboardingInk = Color(0xFF111111);
const _onboardingMutedInk = Color(0xB3111111);

class PairingPage extends HookConsumerWidget {
  /// When true, the pairing page is being used to add a new community
  /// (user is already authenticated with at least one community).
  final bool addingCommunity;

  const PairingPage({super.key, this.addingCommunity = false});

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
    final pairingAppBar = addingCommunity
        ? AppBar(
            foregroundColor: isVerifyingSas
                ? context.colors.onSurface
                : _onboardingInk,
            systemOverlayStyle: isVerifyingSas
                ? null
                : SystemUiOverlayStyle.dark.copyWith(
                    statusBarColor: Colors.transparent,
                  ),
            leading: IconButton(
              icon: const Icon(LucideIcons.arrowLeft),
              onPressed: () => Navigator.of(context).pop(),
            ),
            title: Text(
              'Add Community',
              style: isVerifyingSas
                  ? null
                  : context.textTheme.titleMedium?.copyWith(
                      color: _onboardingInk,
                    ),
            ),
          )
        : null;

    final pairingScaffold = isVerifyingSas
        ? Scaffold(
            backgroundColor: context.colors.surface,
            appBar: pairingAppBar,
            body: SafeArea(
              child: Padding(
                padding: const EdgeInsets.symmetric(horizontal: Grid.sm),
                child: _SasVerificationView(
                  sasCode: pairingState.sasCode ?? '------',
                  confirmed: pairingState.userConfirmedSas,
                  onConfirm: () =>
                      ref.read(pairingProvider.notifier).confirmSas(),
                  onDeny: () => ref.read(pairingProvider.notifier).denySas(),
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
                        ref.read(pairingProvider.notifier).pair(code);
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

class _OnboardingBackground extends StatelessWidget {
  final Widget child;

  const _OnboardingBackground({required this.child});

  @override
  Widget build(BuildContext context) {
    return DecoratedBox(
      key: const Key('pairing-onboarding-background'),
      decoration: const BoxDecoration(
        gradient: LinearGradient(
          begin: Alignment.topCenter,
          end: Alignment.bottomCenter,
          colors: [_onboardingChartreuse, _onboardingShellBottom],
        ),
      ),
      child: CustomPaint(painter: const _DotGridPainter(), child: child),
    );
  }
}

class _DotGridPainter extends CustomPainter {
  const _DotGridPainter();

  @override
  void paint(Canvas canvas, Size size) {
    final dotPaint = Paint()..color = _onboardingInk.withValues(alpha: 0.08);
    const spacing = 24.0;

    for (var x = 0.0; x <= size.width; x += spacing) {
      for (var y = 0.0; y <= size.height; y += spacing) {
        canvas.drawCircle(Offset(x, y), 1, dotPaint);
      }
    }
  }

  @override
  bool shouldRepaint(_DotGridPainter oldDelegate) => false;
}

class _PairingWelcomeView extends StatelessWidget {
  final TextEditingController codeController;
  final bool isBusy;
  final bool pairingCodeExpanded;
  final String? errorMessage;
  final VoidCallback onScan;
  final VoidCallback onTogglePairingCode;
  final VoidCallback onConnect;

  const _PairingWelcomeView({
    required this.codeController,
    required this.isBusy,
    required this.pairingCodeExpanded,
    required this.errorMessage,
    required this.onScan,
    required this.onTogglePairingCode,
    required this.onConnect,
  });

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final revealDuration = reducedMotion
        ? Duration.zero
        : const Duration(milliseconds: 220);

    return LayoutBuilder(
      builder: (context, constraints) {
        return SingleChildScrollView(
          padding: const EdgeInsets.fromLTRB(
            Grid.gutter,
            Grid.sm,
            Grid.gutter,
            Grid.sm,
          ),
          child: ConstrainedBox(
            constraints: BoxConstraints(
              minHeight: constraints.maxHeight - (Grid.sm * 2),
            ),
            child: Column(
              mainAxisAlignment: MainAxisAlignment.center,
              children: [
                Container(
                  width: 136,
                  height: 136,
                  alignment: Alignment.center,
                  decoration: const BoxDecoration(
                    shape: BoxShape.circle,
                    color: Color(0x4DFFFFFF),
                  ),
                  child: const TappableFlappingBee(
                    width: 76,
                    color: _onboardingInk,
                  ),
                ),
                const SizedBox(height: Grid.sm),
                Text(
                  'Welcome to Buzz',
                  textAlign: TextAlign.center,
                  style: context.textTheme.headlineSmall?.copyWith(
                    color: _onboardingInk,
                    fontWeight: FontWeight.w600,
                    letterSpacing: -0.4,
                  ),
                ),
                const SizedBox(height: Grid.xxs),
                Text(
                  'Scan the QR code from your desktop app\nor paste a pairing code to connect.',
                  textAlign: TextAlign.center,
                  style: context.textTheme.bodyMedium?.copyWith(
                    color: _onboardingMutedInk,
                  ),
                ),
                const SizedBox(height: Grid.md),
                Container(
                  width: double.infinity,
                  padding: const EdgeInsets.all(Grid.xs),
                  decoration: BoxDecoration(
                    color: Colors.white.withValues(alpha: 0.58),
                    borderRadius: BorderRadius.circular(Radii.dialog),
                    border: Border.all(
                      color: _onboardingInk.withValues(alpha: 0.08),
                    ),
                  ),
                  child: Column(
                    children: [
                      SizedBox(
                        width: double.infinity,
                        child: FilledButton(
                          style: _onboardingButtonStyle,
                          onPressed: isBusy ? null : onScan,
                          child: isBusy && !pairingCodeExpanded
                              ? const SizedBox(
                                  width: 20,
                                  height: 20,
                                  child: CircularProgressIndicator(
                                    strokeWidth: 2,
                                    color: _onboardingCtaLabel,
                                  ),
                                )
                              : const Row(
                                  mainAxisSize: MainAxisSize.min,
                                  children: [
                                    Icon(LucideIcons.scanLine),
                                    SizedBox(width: Grid.xxs),
                                    Text('Scan a QR code'),
                                  ],
                                ),
                        ),
                      ),
                      const SizedBox(height: Grid.twelve),
                      SizedBox(
                        width: double.infinity,
                        child: OutlinedButton(
                          style: _onboardingSecondaryButtonStyle,
                          onPressed: isBusy ? null : onTogglePairingCode,
                          child: Text(
                            pairingCodeExpanded
                                ? 'Hide pairing code'
                                : 'Use pairing code',
                          ),
                        ),
                      ),
                      AnimatedSwitcher(
                        duration: revealDuration,
                        switchInCurve: Curves.easeOutCubic,
                        switchOutCurve: Curves.easeInCubic,
                        transitionBuilder: (child, animation) {
                          return SizeTransition(
                            sizeFactor: animation,
                            axisAlignment: -1,
                            child: FadeTransition(
                              opacity: animation,
                              child: child,
                            ),
                          );
                        },
                        child: pairingCodeExpanded
                            ? Column(
                                key: const ValueKey('pairing-code-fields'),
                                children: [
                                  const SizedBox(height: Grid.twelve),
                                  TextField(
                                    controller: codeController,
                                    style: context.textTheme.bodyMedium
                                        ?.copyWith(color: _onboardingInk),
                                    cursorColor: _onboardingInk,
                                    decoration: InputDecoration(
                                      filled: true,
                                      fillColor: Colors.white.withValues(
                                        alpha: 0.7,
                                      ),
                                      hintText: 'nostrpair://... or buzz://...',
                                      hintStyle: context.textTheme.bodyMedium
                                          ?.copyWith(
                                            color: _onboardingMutedInk,
                                          ),
                                      prefixIcon: const Icon(
                                        LucideIcons.link,
                                        color: _onboardingInk,
                                      ),
                                      enabledBorder: _inputBorder,
                                      disabledBorder: _inputBorder,
                                      focusedBorder: _inputBorder.copyWith(
                                        borderSide: const BorderSide(
                                          color: _onboardingInk,
                                        ),
                                      ),
                                      isDense: true,
                                    ),
                                    autocorrect: false,
                                    enableSuggestions: false,
                                    enabled: !isBusy,
                                    contextMenuBuilder:
                                        (context, editableTextState) {
                                          return AdaptiveTextSelectionToolbar.editableText(
                                            editableTextState:
                                                editableTextState,
                                          );
                                        },
                                  ),
                                  const SizedBox(height: Grid.twelve),
                                  SizedBox(
                                    width: double.infinity,
                                    child: FilledButton(
                                      style: _onboardingButtonStyle,
                                      onPressed: isBusy ? null : onConnect,
                                      child: isBusy
                                          ? const SizedBox(
                                              width: 20,
                                              height: 20,
                                              child: CircularProgressIndicator(
                                                strokeWidth: 2,
                                                color: _onboardingCtaLabel,
                                              ),
                                            )
                                          : const Text('Connect'),
                                    ),
                                  ),
                                ],
                              )
                            : const SizedBox(
                                key: ValueKey('pairing-code-fields-hidden'),
                              ),
                      ),
                      if (errorMessage != null) ...[
                        const SizedBox(height: Grid.twelve),
                        Container(
                          padding: const EdgeInsets.all(Grid.twelve),
                          decoration: BoxDecoration(
                            color: context.colors.errorContainer,
                            borderRadius: BorderRadius.circular(Radii.md),
                          ),
                          child: Row(
                            children: [
                              Icon(
                                LucideIcons.triangleAlert,
                                size: 16,
                                color: context.colors.onErrorContainer,
                              ),
                              const SizedBox(width: Grid.xxs),
                              Expanded(
                                child: Text(
                                  errorMessage!,
                                  style: context.textTheme.bodySmall?.copyWith(
                                    color: context.colors.onErrorContainer,
                                  ),
                                ),
                              ),
                            ],
                          ),
                        ),
                      ],
                    ],
                  ),
                ),
              ],
            ),
          ),
        );
      },
    );
  }
}

final _inputBorder = OutlineInputBorder(
  borderRadius: BorderRadius.circular(Radii.md),
  borderSide: BorderSide(color: _onboardingInk.withValues(alpha: 0.18)),
);

final _onboardingButtonStyle = FilledButton.styleFrom(
  minimumSize: const Size(0, 48),
  backgroundColor: _onboardingInk,
  foregroundColor: _onboardingCtaLabel,
  disabledBackgroundColor: _onboardingInk.withValues(alpha: 0.38),
  disabledForegroundColor: _onboardingCtaLabel.withValues(alpha: 0.7),
  shape: const StadiumBorder(),
);

final _onboardingSecondaryButtonStyle = OutlinedButton.styleFrom(
  minimumSize: const Size(0, 48),
  backgroundColor: Colors.white.withValues(alpha: 0.28),
  foregroundColor: _onboardingInk,
  disabledBackgroundColor: Colors.white.withValues(alpha: 0.14),
  disabledForegroundColor: _onboardingInk.withValues(alpha: 0.45),
  side: BorderSide(color: _onboardingInk.withValues(alpha: 0.18)),
  shape: const StadiumBorder(),
);

/// SAS verification screen shown during NIP-AB pairing.
class _SasVerificationView extends StatelessWidget {
  final String sasCode;
  final bool confirmed;
  final VoidCallback onConfirm;
  final VoidCallback onDeny;

  const _SasVerificationView({
    required this.sasCode,
    required this.confirmed,
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
          'You are about to transfer your Buzz identity\nto this device. Only confirm if you initiated\nthis pairing from your desktop.',
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
              SizedBox(
                width: 20,
                height: 20,
                child: CircularProgressIndicator(
                  strokeWidth: 2,
                  color: context.colors.primary,
                ),
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
