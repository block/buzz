part of '../add_community_page.dart';

/// The link choice and Continue share one button throughout the transition.
class _AddCommunityConnection extends HookWidget {
  const _AddCommunityConnection({
    super.key,
    required this.enteringLink,
    required this.scannerProgress,
    required this.onBack,
    required this.onCommunityLink,
    required this.onPairing,
    required this.onScan,
    required this.onOpenCommunity,
    required this.busy,
    required this.canSubmit,
    this.errorMessage,
    this.accountStatus,
    this.onInputChanged,
  });

  final bool enteringLink;
  final double scannerProgress;
  final VoidCallback onBack;
  final VoidCallback? onCommunityLink;
  final VoidCallback? onPairing;
  final VoidCallback? onScan;
  final ValueChanged<Uri> onOpenCommunity;
  final bool busy;
  final bool canSubmit;
  final String? errorMessage;
  final String? accountStatus;
  final VoidCallback? onInputChanged;

  @override
  Widget build(BuildContext context) {
    final controller = useTextEditingController();
    final focus = useFocusNode();
    final error = useState<String?>(null);
    final pairingVisible = useState(false);
    void revealPairing() => pairingVisible.value = !pairingVisible.value;
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final ios = defaultTargetPlatform == TargetPlatform.iOS;
    final keyboardReady = useCommunityLinkFocus(
      context,
      enteringLink: enteringLink,
      focus: focus,
    );

    void submit() {
      if (busy || !canSubmit) return;
      final link = parseCommunityLinkInput(controller.text.trim());
      if (link == null) {
        error.value = 'Enter a valid community address or invite link.';
        return;
      }
      focus.unfocus();
      onOpenCommunity(link);
    }

    return _PairingRevealSurface(
      onReveal: enteringLink || busy || scannerProgress > 0
          ? null
          : revealPairing,
      child: TweenAnimationBuilder<double>(
        tween: Tween(end: enteringLink && (!ios || keyboardReady) ? 1 : 0),
        duration: reducedMotion
            ? Duration.zero
            : Duration(milliseconds: ios ? 200 : 150),
        curve: ios ? Curves.easeOutCubic : Curves.easeOutQuad,
        builder: (context, progress, _) => OnboardingStepPage(
          title: enteringLink ? 'Enter a community link' : 'Add a community',
          subtitle: enteringLink
              ? 'Paste an invite link or community address.'
              : accountStatus ?? 'Scan a QR code or enter a community link.',
          onBack: onBack,
          foregroundProgress: scannerProgress,
          keyboardBottomSpacing: Grid.twelve,
          onConnectionOptions: enteringLink || busy ? null : revealPairing,
          content: ExcludeSemantics(
            excluding: !enteringLink,
            child: ExcludeFocus(
              excluding: !enteringLink,
              child: IgnorePointer(
                ignoring: !enteringLink,
                child: Opacity(
                  opacity: progress,
                  child: Transform.translate(
                    key: const Key('add-community-link-rise'),
                    offset: Offset(
                      0,
                      reducedMotion ? 0 : Grid.xl * (1 - progress),
                    ),
                    child: ios
                        ? Column(
                            crossAxisAlignment: CrossAxisAlignment.stretch,
                            children: [
                              IosCommunityLinkInput(
                                controller: controller,
                                focusNode: focus,
                                enabled: !busy,
                                onChanged: (_) {
                                  error.value = null;
                                  onInputChanged?.call();
                                },
                                onSubmitted: (_) => submit(),
                              ),
                              if (error.value ?? errorMessage
                                  case final problem?)
                                Padding(
                                  padding: const EdgeInsets.only(top: Grid.xs),
                                  child: Text(
                                    problem,
                                    style: TextStyle(
                                      color: context.colors.error,
                                    ),
                                  ),
                                ),
                            ],
                          )
                        : TextField(
                            key: const Key('community-link-input'),
                            controller: controller,
                            focusNode: focus,
                            enabled: !busy,
                            keyboardType: TextInputType.url,
                            textInputAction: TextInputAction.go,
                            autocorrect: false,
                            enableSuggestions: false,
                            decoration: InputDecoration(
                              labelText: 'Community link or address',
                              hintText: 'community.example.com',
                              errorText: error.value ?? errorMessage,
                            ),
                            onChanged: (_) {
                              error.value = null;
                              onInputChanged?.call();
                            },
                            onSubmitted: (_) => submit(),
                          ),
                  ),
                ),
              ),
            ),
          ),
          actions: [
            ExcludeSemantics(
              excluding: enteringLink,
              child: ExcludeFocus(
                excluding: enteringLink,
                child: IgnorePointer(
                  ignoring: enteringLink,
                  child: Opacity(
                    opacity: 1 - progress,
                    child: FilledButton(
                      onPressed: busy ? null : onScan,
                      child: const Text('Scan a QR code'),
                    ),
                  ),
                ),
              ),
            ),
            FilledButton(
              key: const Key('add-community-link-action'),
              style: FilledButton.styleFrom(
                backgroundColor: Color.lerp(
                  context.colors.surface,
                  context.colors.primary,
                  progress,
                ),
                foregroundColor: Color.lerp(
                  context.colors.primary,
                  context.colors.onPrimary,
                  progress,
                ),
                side: BorderSide(
                  color: context.colors.outline.withValues(alpha: 1 - progress),
                ),
              ),
              onPressed: busy || (enteringLink && !canSubmit)
                  ? null
                  : enteringLink
                  ? submit
                  : onCommunityLink,
              child: AnimatedSwitcher(
                duration: reducedMotion
                    ? Duration.zero
                    : const Duration(milliseconds: 150),
                child: Text(
                  enteringLink
                      ? busy
                            ? 'Connecting…'
                            : 'Continue'
                      : 'Enter a community link',
                  key: ValueKey(enteringLink),
                ),
              ),
            ),
            if (pairingVisible.value && !enteringLink)
              OutlinedButton(
                onPressed: busy ? null : onPairing,
                child: const Text('Use a pairing code'),
              ),
          ],
        ),
      ),
    );
  }
}
