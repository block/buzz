import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import 'pairing_provider.dart';
import 'pairing_qr_scanner.dart';

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

    final appSurface = PopScope(
      onPopInvokedWithResult: (didPop, _) {
        if (didPop) {
          ref.read(pairingProvider.notifier).reset();
        }
      },
      child: Scaffold(
        appBar: addingCommunity
            ? AppBar(
                leading: IconButton(
                  icon: const Icon(LucideIcons.arrowLeft),
                  onPressed: () => Navigator.of(context).pop(),
                ),
                title: const Text('Add Community'),
              )
            : null,
        body: SafeArea(
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.sm),
            child: pairingState.status == PairingStatus.confirmingSas
                ? _SasVerificationView(
                    sasCode: pairingState.sasCode ?? '------',
                    confirmed: pairingState.userConfirmedSas,
                    onConfirm: () =>
                        ref.read(pairingProvider.notifier).confirmSas(),
                    onDeny: () => ref.read(pairingProvider.notifier).denySas(),
                  )
                : Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    children: [
                      const Spacer(flex: 2),

                      Image.asset('assets/images/buzz-icon.png', height: 64),
                      const SizedBox(height: Grid.xs),
                      Text(
                        'Welcome to Buzz',
                        style: context.textTheme.headlineSmall,
                      ),
                      const SizedBox(height: Grid.xxs),
                      Text(
                        'Scan the QR code from your desktop app\nor paste a pairing code to connect.',
                        textAlign: TextAlign.center,
                        style: context.textTheme.bodyMedium?.copyWith(
                          color: context.colors.onSurfaceVariant,
                        ),
                      ),

                      const SizedBox(height: Grid.lg),

                      // Scan QR button
                      FilledButton.icon(
                        onPressed: isBusy ? null : openScanner,
                        icon: const Icon(LucideIcons.scanLine),
                        label: const Text('Scan QR Code'),
                      ),

                      const SizedBox(height: Grid.sm),

                      Row(
                        children: [
                          const Expanded(child: Divider()),
                          Padding(
                            padding: const EdgeInsets.symmetric(
                              horizontal: Grid.twelve,
                            ),
                            child: Text(
                              'or paste pairing code',
                              style: context.textTheme.bodySmall?.copyWith(
                                color: context.colors.onSurfaceVariant,
                              ),
                            ),
                          ),
                          const Expanded(child: Divider()),
                        ],
                      ),

                      const SizedBox(height: Grid.sm),

                      // Paste field
                      TextField(
                        controller: codeController,
                        decoration: const InputDecoration(
                          hintText: 'nostrpair://... or buzz://...',
                          prefixIcon: Icon(LucideIcons.link),
                          isDense: true,
                        ),
                        autocorrect: false,
                        enableSuggestions: false,
                        enabled: !isBusy,
                        contextMenuBuilder: (context, editableTextState) {
                          return AdaptiveTextSelectionToolbar.editableText(
                            editableTextState: editableTextState,
                          );
                        },
                      ),

                      const SizedBox(height: Grid.twelve),

                      // Connect button
                      SizedBox(
                        width: double.infinity,
                        child: FilledButton(
                          onPressed: isBusy
                              ? null
                              : () {
                                  final code = codeController.text.trim();
                                  if (code.isNotEmpty) {
                                    ref
                                        .read(pairingProvider.notifier)
                                        .pair(code);
                                  }
                                },
                          child: isBusy
                              ? const SizedBox(
                                  width: 20,
                                  height: 20,
                                  child: CircularProgressIndicator(
                                    strokeWidth: 2,
                                    color: Colors.white,
                                  ),
                                )
                              : const Text('Connect'),
                        ),
                      ),

                      // Error message
                      if (pairingState.status == PairingStatus.error &&
                          pairingState.errorMessage != null) ...[
                        const SizedBox(height: Grid.twelve),
                        Container(
                          padding: const EdgeInsets.all(Grid.twelve),
                          decoration: BoxDecoration(
                            color: context.colors.errorContainer,
                            borderRadius: BorderRadius.circular(8),
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
                                  pairingState.errorMessage!,
                                  style: context.textTheme.bodySmall?.copyWith(
                                    color: context.colors.onErrorContainer,
                                  ),
                                ),
                              ),
                            ],
                          ),
                        ),
                      ],

                      const Spacer(flex: 3),
                    ],
                  ),
          ),
        ),
      ),
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
