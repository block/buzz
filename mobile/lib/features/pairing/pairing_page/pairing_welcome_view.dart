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

    return Padding(
      padding: const EdgeInsets.symmetric(
        horizontal: Grid.gutter,
        vertical: Grid.sm,
      ),
      child: CustomScrollView(
        // Stay fixed when the welcome content fits; allow only real overflow
        // (such as the pairing form above the keyboard), with no rubber banding.
        physics: const ClampingScrollPhysics(),
        slivers: [
          SliverFillRemaining(
            hasScrollBody: false,
            child: Column(
              children: [
                Expanded(
                  child: Center(
                    child: Column(
                      mainAxisSize: MainAxisSize.min,
                      children: [
                        ConstrainedBox(
                          constraints: const BoxConstraints(maxWidth: 320),
                          child: AspectRatio(
                            aspectRatio: 777 / 326,
                            child: OnboardingWordmark(
                              key: const Key('pairing-buzz-wordmark'),
                              color: context._onboardingIsDark
                                  ? context._onboardingInk
                                  : null,
                            ),
                          ),
                        ),
                        const SizedBox(height: Grid.xxs),
                        Text(
                          'Your people, your agents, your projects —\nall in one place.',
                          textAlign: TextAlign.center,
                          style: context.textTheme.bodyLarge?.copyWith(
                            color: context._onboardingInk,
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
                const SizedBox(height: Grid.md),
                Text(
                  'Scan the QR code from your desktop app\nor paste a pairing code to connect.',
                  textAlign: TextAlign.center,
                  style: context.textTheme.bodyMedium?.copyWith(
                    color: context._onboardingMutedInk,
                  ),
                ),
                const SizedBox(height: Grid.xs),
                ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 440),
                  child: SizedBox(
                    width: double.infinity,
                    child: Column(
                      children: [
                        FilledButton(
                          style: context._onboardingGlassButtonStyle,
                          onPressed: isBusy ? null : onScan,
                          child: isBusy && !pairingCodeExpanded
                              ? SizedBox(
                                  width: 20,
                                  height: 20,
                                  child: BuzzLoadingIndicator(
                                    size: 20,
                                    color: context._onboardingInk,
                                    semanticLabel: 'Opening scanner',
                                  ),
                                )
                              : const Text('Scan a QR code'),
                        ),
                        const SizedBox(height: Grid.xxs),
                        TextButton(
                          style: context._onboardingGhostButtonStyle,
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
                                          ?.copyWith(
                                            color: context._onboardingInk,
                                          ),
                                      cursorColor: context._onboardingInk,
                                      decoration: InputDecoration(
                                        filled: true,
                                        fillColor:
                                            context._onboardingInputSurface,
                                        hintText:
                                            'nostrpair://... or buzz://...',
                                        hintStyle: context.textTheme.bodyMedium
                                            ?.copyWith(
                                              color:
                                                  context._onboardingMutedInk,
                                            ),
                                        prefixIcon: Icon(
                                          LucideIcons.link,
                                          color: context._onboardingInk,
                                        ),
                                        enabledBorder: context._inputBorder,
                                        disabledBorder: context._inputBorder,
                                        focusedBorder: context._inputBorder
                                            .copyWith(
                                              borderSide: BorderSide(
                                                color: context._onboardingInk,
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
                                        style:
                                            context._onboardingGlassButtonStyle,
                                        onPressed: isBusy ? null : onConnect,
                                        child: isBusy
                                            ? SizedBox(
                                                width: 20,
                                                height: 20,
                                                child: BuzzLoadingIndicator(
                                                  size: 20,
                                                  color: context._onboardingInk,
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
        ],
      ),
    );
  }
}
