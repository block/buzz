part of '../channels_page.dart';

const _kMorphOpenDuration = Duration(milliseconds: 280);
const _kMorphCloseDuration = Duration(milliseconds: 240);
const _kMorphFadeDuration = Duration(milliseconds: 200);
const _kMorphCloseCurve = Cubic(0.22, 1, 0.36, 1);
const double _kMorphOpenBounce = 0.14;
const double _kMorphCloseBounce = 0.06;
const double _kMorphClosedSize = 56;
const double _kMorphOpenHeight = 160;
const double _kMorphOpenRadius = 20;
const double _kMorphSlide = 40;
const double _kMorphScale = 0.97;
const double _kMorphBlur = 2;
const double _kQuickActionCardOverlayOpacity = 0.1;
const double _kQuickActionCardRadius = _kMorphOpenRadius - Grid.xxs;

class _MorphingQuickActionsButton extends HookWidget {
  final bool open;
  final double openEdgeOffset;
  final VoidCallback onToggle;
  final ValueChanged<_QuickAction> onSelected;

  const _MorphingQuickActionsButton({
    required this.open,
    required this.openEdgeOffset,
    required this.onToggle,
    required this.onSelected,
  });

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.of(context).disableAnimations;
    final mediaQuery = MediaQuery.of(context);
    final openWidth = (mediaQuery.size.width - (Grid.gutter * 2))
        .clamp(_kMorphClosedSize, double.infinity)
        .toDouble();
    final surfaceController = useAnimationController(
      initialValue: open ? 1 : 0,
      upperBound: 1.02,
    );
    final fadeController = useAnimationController(
      duration: reducedMotion ? Duration.zero : _kMorphFadeDuration,
      reverseDuration: reducedMotion ? Duration.zero : _kMorphFadeDuration,
      initialValue: open ? 1 : 0,
    );
    final fadeAnimation = useMemoized(
      () => CurvedAnimation(
        parent: fadeController,
        curve: _kMorphCloseCurve,
        reverseCurve: _kMorphCloseCurve,
      ),
      [fadeController],
    );
    final animation = useMemoized(
      () => Listenable.merge([surfaceController, fadeAnimation]),
      [surfaceController, fadeAnimation],
    );

    useEffect(() => fadeAnimation.dispose, [fadeAnimation]);

    useEffect(() {
      fadeController.duration = reducedMotion
          ? Duration.zero
          : _kMorphFadeDuration;
      fadeController.reverseDuration = reducedMotion
          ? Duration.zero
          : _kMorphFadeDuration;

      if (reducedMotion) {
        surfaceController.value = open ? 1 : 0;
        fadeController.value = open ? 1 : 0;
      } else {
        final target = open ? 1.0 : 0.0;
        if ((surfaceController.value - target).abs() > 0.001) {
          unawaited(
            surfaceController.animateWith(
              SpringSimulation(
                SpringDescription.withDurationAndBounce(
                  duration: open ? _kMorphOpenDuration : _kMorphCloseDuration,
                  bounce: open ? _kMorphOpenBounce : _kMorphCloseBounce,
                ),
                surfaceController.value,
                target,
                surfaceController.velocity,
                snapToEnd: true,
              ),
            ),
          );
        }
        if (open) {
          unawaited(fadeController.forward());
        } else {
          unawaited(fadeController.reverse());
        }
      }
      return null;
    }, [fadeController, open, reducedMotion, surfaceController]);

    return AnimatedBuilder(
      animation: animation,
      builder: (context, _) {
        final surfaceValue = surfaceController.value;
        final surfaceProgress = surfaceValue.clamp(0.0, 1.0);
        final fadeValue = fadeAnimation.value.clamp(0.0, 1.0);
        final width = lerpDouble(_kMorphClosedSize, openWidth, surfaceValue)!;
        final height = lerpDouble(
          _kMorphClosedSize,
          _kMorphOpenHeight,
          surfaceValue,
        )!;
        final radius = lerpDouble(
          _kMorphClosedSize / 2,
          _kMorphOpenRadius,
          surfaceValue,
        )!;
        final borderRadius = BorderRadius.circular(radius);

        return Transform.translate(
          offset: Offset(openEdgeOffset * surfaceProgress, 0),
          child: SizedBox(
            key: const Key('quick-actions-surface'),
            width: width,
            height: height,
            child: DecoratedBox(
              decoration: BoxDecoration(
                color: context.colors.primary,
                borderRadius: borderRadius,
                boxShadow: [
                  BoxShadow(
                    color: context.colors.shadow.withValues(alpha: 0.24),
                    blurRadius: 12,
                    offset: const Offset(0, 4),
                  ),
                ],
              ),
              child: ClipRRect(
                borderRadius: borderRadius,
                clipBehavior: Clip.antiAlias,
                child: Material(
                  type: MaterialType.transparency,
                  child: Stack(
                    children: [
                      Positioned.fill(
                        child: OverflowBox(
                          alignment: Alignment.bottomRight,
                          minWidth: openWidth,
                          maxWidth: openWidth,
                          minHeight: _kMorphOpenHeight,
                          maxHeight: _kMorphOpenHeight,
                          child: IgnorePointer(
                            ignoring: !open || fadeValue < 0.9,
                            child: ExcludeSemantics(
                              excluding: !open,
                              child: Opacity(
                                opacity: fadeValue,
                                child: ImageFiltered(
                                  imageFilter: ImageFilter.blur(
                                    sigmaX: _kMorphBlur * (1 - fadeValue),
                                    sigmaY: _kMorphBlur * (1 - fadeValue),
                                  ),
                                  child: Transform.translate(
                                    offset: Offset(
                                      _kMorphSlide * (1 - surfaceProgress),
                                      0,
                                    ),
                                    child: Transform.scale(
                                      alignment: Alignment.bottomRight,
                                      scale:
                                          _kMorphScale +
                                          ((1 - _kMorphScale) *
                                              surfaceProgress),
                                      child: _QuickActionsMenu(
                                        onSelected: onSelected,
                                      ),
                                    ),
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ),
                      ),
                      Positioned(
                        right: 0,
                        bottom: 0,
                        width: _kMorphClosedSize,
                        height: _kMorphClosedSize,
                        child: IgnorePointer(
                          ignoring: open,
                          child: ExcludeSemantics(
                            excluding: open,
                            child: Opacity(
                              opacity: 1 - fadeValue,
                              child: ImageFiltered(
                                imageFilter: ImageFilter.blur(
                                  sigmaX: _kMorphBlur * fadeValue,
                                  sigmaY: _kMorphBlur * fadeValue,
                                ),
                                child: Transform.translate(
                                  offset: Offset(
                                    -_kMorphSlide * surfaceProgress,
                                    0,
                                  ),
                                  child: Transform.rotate(
                                    angle: (pi / 4) * surfaceProgress,
                                    child: Transform.scale(
                                      scale:
                                          1 -
                                          ((1 - _kMorphScale) *
                                              surfaceProgress),
                                      child: Tooltip(
                                        message: 'Create or start conversation',
                                        child: Semantics(
                                          button: true,
                                          label: 'Create or start conversation',
                                          expanded: open,
                                          child: InkWell(
                                            customBorder: const CircleBorder(),
                                            onTap: onToggle,
                                            child: Center(
                                              child: Icon(
                                                LucideIcons.plus,
                                                color: context.colors.onPrimary,
                                              ),
                                            ),
                                          ),
                                        ),
                                      ),
                                    ),
                                  ),
                                ),
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
          ),
        );
      },
    );
  }
}

class _QuickActionsMenu extends StatelessWidget {
  final ValueChanged<_QuickAction> onSelected;

  const _QuickActionsMenu({required this.onSelected});

  @override
  Widget build(BuildContext context) {
    return Padding(
      key: const Key('quick-actions-menu'),
      padding: const EdgeInsets.all(Grid.xxs),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
          _QuickActionItem(
            icon: LucideIcons.hash,
            title: 'Create channel',
            key: const Key('quick-action-create-channel-card'),
            onTap: () => onSelected(_QuickAction.createChannel),
          ),
          const SizedBox(height: Grid.xxs),
          _QuickActionItem(
            icon: LucideIcons.messagesSquare,
            title: 'New direct message',
            key: const Key('quick-action-new-dm-card'),
            onTap: () => onSelected(_QuickAction.newDm),
          ),
        ],
      ),
    );
  }
}

class _QuickActionItem extends StatelessWidget {
  final IconData icon;
  final String title;
  final VoidCallback onTap;

  const _QuickActionItem({
    super.key,
    required this.icon,
    required this.title,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final foreground = context.colors.onPrimary;
    final background = Color.alphaBlend(
      foreground.withValues(alpha: _kQuickActionCardOverlayOpacity),
      context.colors.primary,
    );

    return Expanded(
      child: Material(
        color: background,
        borderRadius: BorderRadius.circular(_kQuickActionCardRadius),
        clipBehavior: Clip.antiAlias,
        child: InkWell(
          onTap: onTap,
          child: Padding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.xs),
            child: Row(
              children: [
                Icon(icon, size: 24, color: foreground),
                const SizedBox(width: Grid.xs),
                Expanded(
                  child: Column(
                    mainAxisAlignment: MainAxisAlignment.center,
                    crossAxisAlignment: CrossAxisAlignment.start,
                    children: [
                      Text(
                        title,
                        maxLines: 1,
                        overflow: TextOverflow.ellipsis,
                        style: context.textTheme.bodyLarge?.copyWith(
                          color: foreground,
                          fontWeight: FontWeight.w600,
                        ),
                      ),
                    ],
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
