part of '../pairing_page.dart';

/// Keeps the selected community surface in place as it opens into a form.
class _OnboardingConnectionView extends HookWidget {
  const _OnboardingConnectionView({
    required this.enteringLink,
    required this.onCommunityLink,
    required this.onBack,
    required this.onOpenCommunity,
    required this.onScan,
    required this.onPairing,
  });

  final bool enteringLink;
  final VoidCallback? onCommunityLink;
  final VoidCallback? onBack;
  final ValueChanged<Uri> onOpenCommunity;
  final VoidCallback? onScan;
  final VoidCallback? onPairing;

  double _textProgress(double progress, bool opening) =>
      opening ? ((progress - 0.2) / 0.8).clamp(0.0, 1.0) : progress;

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final ios = defaultTargetPlatform == TargetPlatform.iOS;
    final keyboardReady = useState(false);
    final staggerEntrance = enteringLink && !ios;
    // Match the iOS return motion, starting after keyboard preparation.
    final duration = reducedMotion
        ? Duration.zero
        : Duration(milliseconds: ios ? 200 : 150);
    final controller = useTextEditingController();
    final focus = useFocusNode();
    // Keep the return destination stable while keyboard insets change.
    // A new viewport (for example rotation) gets a fresh measurement.
    final viewport = MediaQuery.sizeOf(context);
    final restingHeight = useMemoized(() => <double?>[null], [viewport]);
    final enteringLinkRef = useRef(enteringLink)..value = enteringLink;
    useEffect(() {
      const channel = MethodChannel('buzz/community_keyboard');
      Timer? fallback;
      var disposed = false;
      void beginOpening() {
        if (disposed || !enteringLinkRef.value || keyboardReady.value) return;
        fallback?.cancel();
        keyboardReady.value = true;
      }

      if (ios && enteringLink) {
        channel.setMethodCallHandler((call) async {
          if (call.method != 'willShow') return;
          beginOpening();
        });
      }
      if (enteringLink) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (disposed || !context.mounted || !enteringLinkRef.value) return;
          focus.requestFocus();
          if (!ios ||
              reducedMotion ||
              MediaQuery.viewInsetsOf(context).bottom > 0) {
            beginOpening();
          } else {
            // Hardware keyboards and older hosts may send no show notification.
            fallback = Timer(const Duration(milliseconds: 350), beginOpening);
          }
        });
      } else {
        focus.unfocus();
        keyboardReady.value = false;
      }
      return () {
        disposed = true;
        fallback?.cancel();
        if (ios && enteringLink) channel.setMethodCallHandler(null);
      };
    }, [enteringLink, ios, reducedMotion]);

    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
      child: Column(
        children: [
          Align(
            alignment: Alignment.centerLeft,
            child: defaultTargetPlatform == TargetPlatform.iOS
                ? IosGlassNavigationButton(
                    key: const Key('community-link-back'),
                    icon: IosGlassNavigationIcon.back,
                    semanticLabel: 'Back',
                    foregroundColor: context.colors.onSurface,
                    onPressed: onBack,
                  )
                : BackButton(
                    key: const Key('community-link-back'),
                    color: context.colors.onSurface,
                    onPressed: onBack,
                  ),
          ),
          Expanded(
            child: LayoutBuilder(
              builder: (context, constraints) {
                if (!enteringLink &&
                    MediaQuery.viewInsetsOf(context).bottom == 0) {
                  restingHeight[0] ??= constraints.maxHeight;
                }
                return TweenAnimationBuilder<double>(
                  tween: Tween(
                    end: enteringLink && (!ios || keyboardReady.value) ? 1 : 0,
                  ),
                  duration: duration,
                  curve: ios
                      ? Curves.easeOutCubic
                      : enteringLink
                      ? Curves.easeOutQuad
                      : Curves.easeOutCubic.flipped,
                  builder: (context, progress, _) => _CommunityLinkForm(
                    controller: controller,
                    focusNode: focus,
                    showBack: false,
                    fullPage: true,
                    autofocus: false,
                    showHeading: false,
                    onOpen: onOpenCommunity,
                    onBack: onBack,
                    layoutBuilder: (fields, action) => CustomMultiChildLayout(
                      delegate: _ConnectionLayout(
                        progress: progress,
                        opening: staggerEntrance,
                        restingHeight: restingHeight[0],
                      ),
                      children: [
                        LayoutId(
                          id: 'heading',
                          child: SingleChildScrollView(
                            child: Stack(
                              children: [
                                _ConnectionMotionLayer(
                                  progress:
                                      1 -
                                      _textProgress(progress, staggerEntrance),
                                  active: !enteringLink,
                                  maintainSize: true,
                                  child: const _ConnectionHeading(
                                    title: 'Connect to a community',
                                    subtitle:
                                        'Scan a QR code or enter a community link to continue.',
                                  ),
                                ),
                                _ConnectionMotionLayer(
                                  progress: _textProgress(
                                    progress,
                                    staggerEntrance,
                                  ),
                                  active: enteringLink,
                                  maintainSize: true,
                                  child: const _ConnectionHeading(
                                    title: 'Enter a community link',
                                    subtitle:
                                        'Paste an invite link or community address.',
                                  ),
                                ),
                              ],
                            ),
                          ),
                        ),
                        LayoutId(
                          id: 'scan',
                          child: _ConnectionMotionLayer(
                            key: const Key('connection-options-motion'),
                            progress: 1 - progress,
                            active: !enteringLink,
                            maintainSize: true,
                            child: _OnboardingChoiceContainer(
                              title: 'Scan a QR code',
                              icon: BuzzIcons.qrcode,
                              subtitle:
                                  'Scan a community or desktop pairing code.',
                              onTap: onScan,
                              onIconDoubleTap: onPairing,
                            ),
                          ),
                        ),
                        LayoutId(
                          id: 'surface',
                          child: DecoratedBox(
                            decoration: BoxDecoration(
                              color: Color.lerp(
                                context.colors.surface,
                                context.colors.primary,
                                progress,
                              ),
                              borderRadius: BorderRadius.circular(
                                Radii.container +
                                    (26 - Radii.container) * progress,
                              ),
                            ),
                          ),
                        ),
                        LayoutId(
                          id: 'link',
                          child: _ConnectionMotionLayer(
                            progress: 1 - progress,
                            active: !enteringLink,
                            maintainSize: true,
                            child: _OnboardingChoiceContainer(
                              title: 'Enter a community link',
                              transparent: true,
                              icon: BuzzIcons.link,
                              subtitle:
                                  'Join a community with its address or invite.',
                              onTap: onCommunityLink,
                            ),
                          ),
                        ),
                        LayoutId(
                          id: 'action',
                          child: _ConnectionMotionLayer(
                            progress: progress,
                            active: enteringLink,
                            maintainSize: true,
                            child: action,
                          ),
                        ),
                        LayoutId(
                          id: 'form',
                          child: _ConnectionMotionLayer(
                            key: const Key('community-form-motion'),
                            maintainSize: true,
                            progress: staggerEntrance
                                ? ((progress - 0.4) / 0.6).clamp(0.0, 1.0)
                                : (progress / 0.8).clamp(0.0, 1.0),
                            active: enteringLink,
                            child: fields,
                          ),
                        ),
                      ],
                    ),
                  ),
                );
              },
            ),
          ),
        ],
      ),
    );
  }
}

/// Measures the cards so the resting heading sits directly above them,
/// independent of text wrapping. Both states share the same moving heading.
class _ConnectionLayout extends MultiChildLayoutDelegate {
  _ConnectionLayout({
    required this.progress,
    required this.opening,
    this.restingHeight,
  });

  final bool opening;

  final double? restingHeight;

  final double progress;

  @override
  void performLayout(Size size) {
    final baselineHeight = restingHeight ?? size.height;
    final heading = layoutChild(
      'heading',
      BoxConstraints(
        minWidth: size.width,
        maxWidth: size.width,
        maxHeight: size.height / 3,
      ),
    );
    final cardConstraints = BoxConstraints(
      minWidth: size.width,
      maxWidth: size.width,
      maxHeight: baselineHeight,
    );
    final scan = layoutChild('scan', cardConstraints);
    final link = layoutChild('link', cardConstraints);
    final restingTop =
        (baselineHeight -
                Grid.sm -
                link.height -
                Grid.xs -
                scan.height -
                Grid.sm -
                heading.height)
            .clamp(0.0, baselineHeight);
    final expandedTop = Grid.sm.toDouble();
    final movement = opening ? (progress / 0.8).clamp(0.0, 1.0) : progress;
    final headingTop = restingTop + (expandedTop - restingTop) * movement;
    positionChild('heading', Offset(0, headingTop));
    positionChild(
      'scan',
      Offset(
        0,
        baselineHeight -
            Grid.sm -
            link.height -
            Grid.xs -
            scan.height +
            Grid.sm * progress,
      ),
    );
    final linkTop = baselineHeight - Grid.sm - link.height;
    final actionTop = size.height - Grid.sm - 52;
    final sharedTop = linkTop + (actionTop - linkTop) * progress;
    positionChild('link', Offset(0, sharedTop));
    layoutChild(
      'surface',
      BoxConstraints.tight(
        Size(size.width, link.height + (52 - link.height) * progress),
      ),
    );
    positionChild('surface', Offset(0, sharedTop));
    layoutChild(
      'action',
      BoxConstraints.tight(
        Size(size.width, link.height + (52 - link.height) * progress),
      ),
    );
    positionChild('action', Offset(0, sharedTop));
    if (hasChild('form')) {
      final formTop = expandedTop + heading.height + Grid.sm;
      layoutChild(
        'form',
        BoxConstraints.tight(
          Size(
            size.width,
            (size.height - formTop - Grid.sm - 52 - Grid.xs).clamp(
              0.0,
              size.height,
            ),
          ),
        ),
      );
      positionChild('form', Offset(0, headingTop + heading.height + Grid.sm));
    }
  }

  @override
  bool shouldRelayout(_ConnectionLayout oldDelegate) =>
      oldDelegate.progress != progress ||
      oldDelegate.opening != opening ||
      oldDelegate.restingHeight != restingHeight;
}

class _ConnectionHeading extends StatelessWidget {
  const _ConnectionHeading({required this.title, required this.subtitle});
  final String title;
  final String subtitle;

  @override
  Widget build(BuildContext context) => Column(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      Text(title, style: context.textTheme.headlineSmall),
      const SizedBox(height: Grid.xs),
      Text(
        subtitle,
        style: context.textTheme.bodyLarge?.copyWith(
          color: context.colors.onSurfaceVariant,
        ),
      ),
    ],
  );
}

class _ConnectionMotionLayer extends StatelessWidget {
  const _ConnectionMotionLayer({
    super.key,
    required this.progress,
    required this.active,
    required this.child,
    this.maintainSize = false,
  });
  final double progress;
  final bool active;
  final Widget child;
  final bool maintainSize;

  @override
  Widget build(BuildContext context) => Visibility(
    visible: active || progress > 0,
    maintainState: true,
    maintainAnimation: true,
    maintainSize: maintainSize,
    child: IgnorePointer(
      ignoring: !active,
      child: ExcludeFocus(
        excluding: !active,
        child: ExcludeSemantics(
          excluding: !active,
          child: Opacity(opacity: progress, child: child),
        ),
      ),
    ),
  );
}

class _OnboardingChoiceContainer extends StatelessWidget {
  const _OnboardingChoiceContainer({
    required this.title,
    required this.subtitle,
    required this.icon,
    this.onTap,
    this.onIconDoubleTap,
    this.transparent = false,
  });

  final String title;
  final String subtitle;
  final IconData icon;
  final bool transparent;
  final VoidCallback? onTap;
  final VoidCallback? onIconDoubleTap;

  @override
  Widget build(BuildContext context) => Semantics(
    button: onTap != null,
    label: title,
    hint: subtitle,
    onTap: onTap,
    child: ExcludeSemantics(
      excluding: true,
      child: Material(
        // White surface cards over the onboarding artwork.
        color: transparent ? Colors.transparent : context.colors.surface,
        borderRadius: BorderRadius.circular(Radii.container),
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          onTap: onTap,
          excludeFromSemantics: true,
          child: Padding(
            padding: const EdgeInsets.all(Grid.gutter),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    ExcludeSemantics(
                      child: GestureDetector(
                        key: onIconDoubleTap == null
                            ? null
                            : const Key('developer-pairing-trigger'),
                        excludeFromSemantics: true,
                        onDoubleTap: onIconDoubleTap,
                        child: SizedBox(
                          width: Grid.sm,
                          height: Grid.sm,
                          child: Icon(
                            icon,
                            size: Grid.sm,
                            color: context.colors.onSurface,
                          ),
                        ),
                      ),
                    ),
                    const SizedBox(width: Grid.xs),
                    Expanded(
                      child: Column(
                        crossAxisAlignment: CrossAxisAlignment.start,
                        children: [
                          // Align the visible glyphs with the icon, accounting
                          // for the title font's leading above its cap height.
                          Transform.translate(
                            offset: const Offset(0, -2),
                            child: Text(
                              title,
                              style: context.textTheme.titleSmall?.copyWith(
                                color: context.colors.onSurface,
                              ),
                            ),
                          ),
                          const SizedBox(height: Grid.half),
                          Text(
                            subtitle,
                            style: context.textTheme.bodyMedium?.copyWith(
                              color: context.colors.onSurfaceVariant,
                            ),
                          ),
                        ],
                      ),
                    ),
                  ],
                ),
              ],
            ),
          ),
        ),
      ),
    ),
  );
}
