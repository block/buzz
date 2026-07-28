part of '../pairing_page.dart';

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
                ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 440),
                  child: SizedBox(
                    width: double.infinity,
                    child: Column(
                      children: [
                        FilledButton(
                          style: _onboardingButtonStyle,
                          onPressed: isBusy ? null : onScan,
                          child: isBusy && !pairingCodeExpanded
                              ? const SizedBox(
                                  width: 20,
                                  height: 20,
                                  child: BuzzLoadingIndicator(
                                    size: 20,
                                    color: _onboardingCtaLabel,
                                    semanticLabel: 'Opening scanner',
                                  ),
                                )
                              : const Text('Scan a QR code'),
                        ),
                        const SizedBox(height: Grid.xxs),
                        TextButton(
                          style: _onboardingSecondaryButtonStyle,
                          onPressed: isBusy ? null : onTogglePairingCode,
                          child: Text(
                            pairingCodeExpanded
                                ? 'Hide pairing code'
                                : 'Use pairing code',
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
                                        hintText:
                                            'nostrpair://... or buzz://...',
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
                                                child: BuzzLoadingIndicator(
                                                  size: 20,
                                                  color: _onboardingCtaLabel,
                                                  semanticLabel: 'Connecting',
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
                                    style: context.textTheme.bodySmall
                                        ?.copyWith(
                                          color:
                                              context.colors.onErrorContainer,
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
  padding: const EdgeInsets.symmetric(
    horizontal: Grid.lg,
    vertical: Grid.twelve,
  ),
  backgroundColor: _onboardingInk,
  foregroundColor: _onboardingCtaLabel,
  disabledBackgroundColor: _onboardingInk.withValues(alpha: 0.38),
  disabledForegroundColor: _onboardingCtaLabel.withValues(alpha: 0.7),
  shape: const StadiumBorder(),
);

final _onboardingSecondaryButtonStyle = TextButton.styleFrom(
  minimumSize: const Size(0, 44),
  padding: const EdgeInsets.symmetric(horizontal: Grid.md, vertical: Grid.xxs),
  backgroundColor: _onboardingInk.withValues(alpha: 0.1),
  foregroundColor: _onboardingInk,
  disabledBackgroundColor: _onboardingInk.withValues(alpha: 0.05),
  disabledForegroundColor: _onboardingInk.withValues(alpha: 0.45),
  shape: const StadiumBorder(),
);
