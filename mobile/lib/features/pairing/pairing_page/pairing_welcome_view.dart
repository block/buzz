part of '../pairing_page.dart';

class _PairingWelcomeView extends HookWidget {
  final ValueNotifier<bool> optionsExpanded;
  final TextEditingController codeController;
  final bool isBusy;
  final bool pairingCodeExpanded;
  final String? errorMessage;
  final VoidCallback onScan;
  final VoidCallback onTogglePairingCode;
  final VoidCallback onConnect;
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
  final ValueChanged<Uri> onOpenCommunity;

  const _PairingWelcomeView({
    required this.optionsExpanded,
    required this.codeController,
    required this.isBusy,
    required this.pairingCodeExpanded,
    required this.errorMessage,
    required this.onScan,
    required this.onTogglePairingCode,
    required this.onConnect,
    required this.onOpenCommunity,
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
  });

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final revealDuration = reducedMotion
        ? Duration.zero
        : const Duration(milliseconds: 150);

    final selectedPage = creatingCommunity
        ? 4
        : enteringCommunityLink
        ? 3
        : existingUser
        ? 2
        : 1;
    List<Widget> connectionActions() => [
      const SizedBox(height: Grid.xxs),
      FilledButton(
        style: context._onboardingGlassButtonStyle,
        onPressed: onCommunityLink,
        child: const Text('Enter a community link'),
      ),
      const SizedBox(height: Grid.xxs),
      GestureDetector(
        key: const Key('developer-pairing-trigger'),
        excludeFromSemantics: true,
        onDoubleTap: isBusy ? null : onAlreadyUseBuzz,
        child: FilledButton(
          style: context._onboardingGlassButtonStyle,
          onPressed: isBusy ? null : onScan,
          child: const Text('Scan a QR code'),
        ),
      ),
    ];
    final showingOptions =
        optionsExpanded.value &&
        !existingUser &&
        setupContent == null &&
        !showConnectionOptions;
    Widget actions(int page) => page == 3 && optionsExpanded.value
        ? const SizedBox.shrink()
        : page == 3 || page == 4
        ? _CommunityLinkForm(
            onOpen: onOpenCommunity,
            onBack: onWelcomeBack,
            onCreate: page == 4 ? onSubmitCommunity : null,
          )
        : Column(
            children: [
              if (page == 1 && accountStatus != null) ...[
                Text(
                  accountStatus!,
                  textAlign: TextAlign.center,
                  style: context.textTheme.bodyMedium?.copyWith(
                    color: context._onboardingMutedInk,
                  ),
                ),
                const SizedBox(height: Grid.xs),
              ],
              if (page == 2) ...[
                Text(
                  connectionLanding
                      ? 'Paste the pairing code from your desktop app.'
                      : 'Scan the QR code from your desktop app\nor paste a pairing code to connect.',
                  textAlign: TextAlign.center,
                  style: context.textTheme.bodyMedium?.copyWith(
                    color: context._onboardingMutedInk,
                  ),
                ),
                const SizedBox(height: Grid.xs),
              ],
              if (page == 1) ...[
                if (HostedEnvironment.enabled)
                  FilledButton(
                    style: context._onboardingGlassButtonStyle,
                    onPressed: onBuilderlab,
                    child: builderlabBusy
                        ? BuzzLoadingIndicator(
                            size: 20,
                            color: context._onboardingInk,
                            semanticLabel: builderlabLabel,
                          )
                        : Text(builderlabLabel),
                  ),
                if (!showConnectionOptions) ...[
                  const SizedBox(height: Grid.xxs),
                  TextButton(
                    style: context._onboardingGhostButtonStyle,
                    onPressed: isBusy
                        ? null
                        : () => optionsExpanded.value = !optionsExpanded.value,
                    child: const Text('Other options'),
                  ),
                ],
                if (showConnectionOptions) ...connectionActions(),
                if (HostedEnvironment.enabled &&
                    showConnectionOptions &&
                    onCreateCommunity != null)
                  TextButton(
                    onPressed: isBusy ? null : onCreateCommunity,
                    child: const Text('Create a community'),
                  ),
              ] else ...[
                if (!connectionLanding) ...[
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
                ],
                AnimatedSwitcher(
                  duration: revealDuration,
                  switchInCurve: Curves.easeOutQuad,
                  switchOutCurve: Curves.easeOutCubic,
                  transitionBuilder: (child, animation) {
                    return SizeTransition(
                      sizeFactor: animation,
                      alignment: Alignment.topCenter,
                      child: FadeTransition(opacity: animation, child: child),
                    );
                  },
                  child: (connectionLanding || pairingCodeExpanded)
                      ? Column(
                          key: const ValueKey('pairing-code-fields'),
                          children: [
                            const SizedBox(height: Grid.twelve),
                            TextField(
                              controller: codeController,
                              style: context.textTheme.bodyMedium?.copyWith(
                                color: context._onboardingInk,
                              ),
                              cursorColor: context._onboardingInk,
                              decoration: InputDecoration(
                                filled: true,
                                fillColor: context._onboardingInputSurface,
                                hintText: 'nostrpair://... or buzz://...',
                                hintStyle: context.textTheme.bodyMedium
                                    ?.copyWith(
                                      color: context._onboardingMutedInk,
                                    ),
                                prefixIcon: Icon(
                                  BuzzIcons.link,
                                  color: context._onboardingInk,
                                ),
                                enabledBorder: context._inputBorder,
                                disabledBorder: context._inputBorder,
                                focusedBorder: context._inputBorder.copyWith(
                                  borderSide: BorderSide(
                                    color: context._onboardingInk,
                                  ),
                                ),
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
                            SizedBox(
                              width: double.infinity,
                              child: FilledButton(
                                style: context._onboardingGlassButtonStyle,
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
              ],
              if (page == 2 && onWelcomeBack != null)
                TextButton(
                  style: context._onboardingGhostButtonStyle,
                  onPressed: onWelcomeBack,
                  child: const Text('Back'),
                ),
              if ((!connectionLanding || page == selectedPage) &&
                  errorMessage != null) ...[
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
                        BuzzIcons.triangleAlert,
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
          );

    final welcome = Padding(
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
                ConstrainedBox(
                  constraints: const BoxConstraints(maxWidth: 440),
                  child: SizedBox(
                    width: double.infinity,
                    child: setupContent != null
                        ? setupContent!
                        : !connectionLanding
                        ? actions(2)
                        : ClipRect(
                            child: Stack(
                              alignment: Alignment.bottomCenter,
                              children: [
                                for (var page = 1; page < 5; page++)
                                  ExcludeFocus(
                                    excluding: page != selectedPage,
                                    child: ExcludeSemantics(
                                      excluding: page != selectedPage,
                                      child: IgnorePointer(
                                        ignoring: page != selectedPage,
                                        child: AnimatedSlide(
                                          key: ValueKey('welcome-panel-$page'),
                                          offset: Offset(
                                            page == selectedPage
                                                ? 0
                                                : page == 1
                                                ? -1
                                                : 1,
                                            0,
                                          ),
                                          duration: revealDuration,
                                          curve: page == selectedPage
                                              ? Curves.easeOutQuad
                                              : Curves.easeOutCubic.flipped,
                                          child: SizedBox(
                                            width: double.infinity,
                                            child: actions(page),
                                          ),
                                        ),
                                      ),
                                    ),
                                  ),
                              ],
                            ),
                          ),
                  ),
                ),
              ],
            ),
          ),
        ],
      ),
    );
    return PopScope(
      canPop: !showingOptions,
      onPopInvokedWithResult: (didPop, _) {
        if (!didPop && showingOptions && !enteringCommunityLink) {
          optionsExpanded.value = false;
        }
      },
      child: TweenAnimationBuilder<double>(
        tween: Tween(end: showingOptions ? 1 : 0),
        duration: reducedMotion
            ? Duration.zero
            : const Duration(milliseconds: 150),
        curve: showingOptions
            ? Curves.easeOutQuad
            : Curves.easeOutCubic.flipped,
        child: welcome,
        builder: (context, progress, child) => Stack(
          fit: StackFit.expand,
          children: [
            ExcludeFocus(
              excluding: showingOptions,
              child: ExcludeSemantics(
                excluding: showingOptions,
                child: IgnorePointer(
                  ignoring: showingOptions,
                  child: Opacity(
                    opacity: 1 - progress,
                    child: ImageFiltered(
                      enabled: !reducedMotion && progress > 0 && progress < 1,
                      imageFilter: ImageFilter.blur(
                        sigmaX: 16 * progress,
                        sigmaY: 16 * progress,
                      ),
                      child: child,
                    ),
                  ),
                ),
              ),
            ),
            if (showingOptions || progress > 0) ...[
              Opacity(
                opacity: progress,
                child: Transform.translate(
                  offset: Offset(0, reducedMotion ? 0 : 16 * (1 - progress)),
                  child: IgnorePointer(
                    ignoring: !showingOptions,
                    child: ExcludeFocus(
                      excluding: !showingOptions,
                      child: ExcludeSemantics(
                        excluding: !showingOptions,
                        child: _OnboardingConnectionView(
                          enteringLink: enteringCommunityLink,
                          onCommunityLink: onCommunityLink,
                          onBack: () {
                            if (enteringCommunityLink) {
                              onWelcomeBack?.call();
                            } else {
                              optionsExpanded.value = false;
                            }
                          },
                          onOpenCommunity: onOpenCommunity,
                          onScan: isBusy ? null : onScan,
                          onPairing: isBusy ? null : onAlreadyUseBuzz,
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ],
          ],
        ),
      ),
    );
  }
}
