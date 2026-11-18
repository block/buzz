part of '../add_community_page.dart';

/// The link choice and Continue share one button throughout the transition.
class _AddCommunityConnection extends HookWidget {
  const _AddCommunityConnection({
    super.key,
    required this.enteringLink,
    required this.creatingCommunity,
    this.onSubmitCommunity,
    required this.onboarding,
    this.onFindCommunities,
    this.onImportIdentity,
    this.onCreateCommunity,
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
  final bool creatingCommunity;
  final ValueChanged<String>? onSubmitCommunity;
  final bool onboarding;
  final VoidCallback? onFindCommunities;
  final VoidCallback? onImportIdentity;
  final VoidCallback? onCreateCommunity;
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
    final linkController = useTextEditingController();
    final createController = useTextEditingController();
    final enteringForm = enteringLink || creatingCommunity;
    final controller = creatingCommunity ? createController : linkController;
    final focus = useFocusNode();
    final error = useState<String?>(null);
    final pairingVisible = useState(false);
    useEffect(() {
      error.value = null;
      return null;
    }, [creatingCommunity, enteringLink]);
    void revealPairing() => pairingVisible.value = !pairingVisible.value;
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final ios = defaultTargetPlatform == TargetPlatform.iOS;
    final keyboardReady = useCommunityLinkFocus(
      context,
      enteringLink: enteringForm,
      focus: focus,
    );

    void submit() {
      if (busy || !canSubmit) return;
      if (creatingCommunity) {
        final name = controller.text.trim().toLowerCase();
        if (!HostedCreateCommunity.validName(name)) {
          error.value =
              'Use letters, numbers and single hyphens (up to 63 characters).';
          return;
        }
        error.value = null;
        focus.unfocus();
        onSubmitCommunity?.call(name);
        return;
      }
      final link = parseCommunityLinkInput(controller.text.trim());
      if (link == null) {
        error.value = 'Enter a valid community address or invite link.';
        return;
      }
      focus.unfocus();
      onOpenCommunity(link);
    }

    return _PairingRevealSurface(
      onReveal: enteringForm || busy || scannerProgress > 0
          ? null
          : revealPairing,
      child: TweenAnimationBuilder<double>(
        tween: Tween(end: enteringForm && (!ios || keyboardReady) ? 1 : 0),
        duration: reducedMotion
            ? Duration.zero
            : Duration(milliseconds: ios ? 200 : 150),
        curve: ios ? Curves.easeOutCubic : Curves.easeOutQuad,
        builder: (context, progress, _) => OnboardingStepPage(
          title: creatingCommunity
              ? 'Create a community'
              : enteringLink
              ? 'Enter a community link'
              : 'Add a community',
          subtitle: creatingCommunity
              ? 'Choose an address for your community.'
              : enteringForm
              ? 'Paste an invite link or community address.'
              : onboarding
              ? accountStatus ??
                    'Join your people or create a space of your own.'
              : accountStatus ?? 'Scan a QR code or enter a community link.',
          onBack: onBack,
          foregroundProgress: scannerProgress,
          keyboardBottomSpacing: Grid.twelve,
          onConnectionOptions: enteringForm || busy ? null : revealPairing,
          content: ExcludeSemantics(
            excluding: !enteringForm,
            child: ExcludeFocus(
              excluding: !enteringForm,
              child: IgnorePointer(
                ignoring: !enteringForm,
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
                                inputKey: Key(
                                  creatingCommunity
                                      ? 'community-create-input'
                                      : 'community-link-input',
                                ),
                                placeholder: creatingCommunity
                                    ? 'Community address'
                                    : 'Community link or address',
                                focusNode: focus,
                                enabled: !busy,
                                onChanged: (_) {
                                  error.value = null;
                                  onInputChanged?.call();
                                },
                                onSubmitted: (_) => submit(),
                              ),
                              if (creatingCommunity)
                                Text(
                                  '.${HostedEnvironment.communitySuffix}',
                                  style: context.textTheme.bodyMedium?.copyWith(
                                    color: context.colors.onSurfaceVariant,
                                  ),
                                ),
                              if (error.value ?? errorMessage
                                  case final problem?)
                                Padding(
                                  padding: const EdgeInsets.only(top: Grid.xs),
                                  child: Semantics(
                                    liveRegion: true,
                                    child: Text(
                                      problem,
                                      style: TextStyle(
                                        color: context.colors.error,
                                      ),
                                    ),
                                  ),
                                ),
                            ],
                          )
                        : TextField(
                            key: Key(
                              creatingCommunity
                                  ? 'community-create-input'
                                  : 'community-link-input',
                            ),
                            controller: controller,
                            focusNode: focus,
                            enabled: !busy,
                            keyboardType: TextInputType.url,
                            textInputAction: TextInputAction.go,
                            autocorrect: false,
                            enableSuggestions: false,
                            decoration: InputDecoration(
                              labelText: creatingCommunity
                                  ? 'Community address'
                                  : 'Community link or address',
                              hintText: creatingCommunity
                                  ? 'my-community'
                                  : 'community.example.com',
                              helperText: creatingCommunity
                                  ? '.${HostedEnvironment.communitySuffix}'
                                  : null,
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
            if (!enteringForm && errorMessage != null)
              Semantics(
                liveRegion: true,
                child: Text(
                  errorMessage!,
                  style: TextStyle(color: context.colors.error),
                ),
              ),
            ExcludeSemantics(
              excluding: enteringForm,
              child: ExcludeFocus(
                excluding: enteringForm,
                child: IgnorePointer(
                  ignoring: enteringForm,
                  child: Opacity(
                    opacity: 1 - progress,
                    child: FilledButton(
                      onPressed: busy ? null : onScan,
                      onLongPress: busy || scannerProgress > 0
                          ? null
                          : revealPairing,
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
                  context.controlSurface,
                  context.colors.primary,
                  progress,
                ),
                foregroundColor: Color.lerp(
                  context.colors.primary,
                  context.colors.onPrimary,
                  progress,
                ),
                side: BorderSide.none,
              ),
              onPressed: busy || (enteringForm && !canSubmit)
                  ? null
                  : enteringForm
                  ? submit
                  : onCommunityLink,
              child: AnimatedSwitcher(
                duration: reducedMotion
                    ? Duration.zero
                    : const Duration(milliseconds: 150),
                child: Text(
                  enteringForm
                      ? busy
                            ? 'Connecting…'
                            : 'Continue'
                      : onboarding
                      ? 'Enter community link'
                      : 'Enter a community link',
                  key: ValueKey(enteringForm),
                ),
              ),
            ),
            if (!enteringForm) ...[
              if (onFindCommunities != null)
                FilledButton(
                  style: FilledButton.styleFrom(
                    backgroundColor: context.controlSurface,
                    foregroundColor: context.colors.onSurface,
                  ),
                  onPressed: busy ? null : onFindCommunities,
                  child: const Text('Find my communities'),
                ),
              if (onImportIdentity != null)
                TextButton(
                  onPressed: busy ? null : onImportIdentity,
                  child: const Text('Use an existing identity'),
                ),
              if (onboarding && onCreateCommunity != null)
                FilledButton(
                  style: FilledButton.styleFrom(
                    backgroundColor: context.controlSurface,
                    foregroundColor: context.colors.onSurface,
                  ),
                  onPressed: busy ? null : onCreateCommunity,
                  child: const Text('Create a community'),
                ),
            ],
            if (pairingVisible.value && !enteringForm)
              TextButton(
                onPressed: busy ? null : onPairing,
                child: const Text('Use a pairing code'),
              ),
          ],
        ),
      ),
    );
  }
}
