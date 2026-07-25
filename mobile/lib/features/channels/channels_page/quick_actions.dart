part of '../channels_page.dart';

const _kMorphOpenDuration = Duration(milliseconds: 350);
const _kMorphCloseDuration = Duration(milliseconds: 250);
const _kMorphFadeDuration = Duration(milliseconds: 200);
const _kMorphOpenCurve = Cubic(0.34, 1.25, 0.64, 1);
const _kMorphCloseCurve = Cubic(0.22, 1, 0.36, 1);
const double _kMorphClosedSize = 56;
const double _kMorphOpenHeight = 160;
const double _kMorphOpenRadius = 20;
const double _kMorphSlide = 40;
const double _kMorphScale = 0.97;
const double _kMorphBlur = 2;

class _MorphingQuickActionsButton extends HookWidget {
  final bool open;
  final VoidCallback onToggle;
  final ValueChanged<_QuickAction> onSelected;

  const _MorphingQuickActionsButton({
    required this.open,
    required this.onToggle,
    required this.onSelected,
  });

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.of(context).disableAnimations;
    final mediaQuery = MediaQuery.of(context);
    final openWidth =
        (mediaQuery.size.width - mediaQuery.padding.horizontal - (Grid.xs * 2))
            .clamp(_kMorphClosedSize, double.infinity)
            .toDouble();
    final surfaceController = useAnimationController(
      duration: reducedMotion ? Duration.zero : _kMorphOpenDuration,
      reverseDuration: reducedMotion ? Duration.zero : _kMorphCloseDuration,
      initialValue: open ? 1 : 0,
    );
    final fadeController = useAnimationController(
      duration: reducedMotion ? Duration.zero : _kMorphFadeDuration,
      reverseDuration: reducedMotion ? Duration.zero : _kMorphFadeDuration,
      initialValue: open ? 1 : 0,
    );
    final surfaceAnimation = useMemoized(
      () => CurvedAnimation(
        parent: surfaceController,
        curve: _kMorphOpenCurve,
        reverseCurve: _kMorphCloseCurve,
      ),
      [surfaceController],
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
      () => Listenable.merge([surfaceAnimation, fadeAnimation]),
      [surfaceAnimation, fadeAnimation],
    );

    useEffect(() => surfaceAnimation.dispose, [surfaceAnimation]);
    useEffect(() => fadeAnimation.dispose, [fadeAnimation]);

    useEffect(() {
      surfaceController.duration = reducedMotion
          ? Duration.zero
          : _kMorphOpenDuration;
      surfaceController.reverseDuration = reducedMotion
          ? Duration.zero
          : _kMorphCloseDuration;
      fadeController.duration = reducedMotion
          ? Duration.zero
          : _kMorphFadeDuration;
      fadeController.reverseDuration = reducedMotion
          ? Duration.zero
          : _kMorphFadeDuration;

      if (reducedMotion) {
        surfaceController.value = open ? 1 : 0;
        fadeController.value = open ? 1 : 0;
      } else if (open) {
        unawaited(surfaceController.forward());
        unawaited(fadeController.forward());
      } else {
        unawaited(surfaceController.reverse());
        unawaited(fadeController.reverse());
      }
      return null;
    }, [fadeController, open, reducedMotion, surfaceController]);

    return AnimatedBuilder(
      animation: animation,
      builder: (context, _) {
        final surfaceValue = surfaceAnimation.value;
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

        return SizedBox(
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
                                    _kMorphSlide * (1 - surfaceValue),
                                    0,
                                  ),
                                  child: Transform.scale(
                                    alignment: Alignment.bottomRight,
                                    scale:
                                        _kMorphScale +
                                        ((1 - _kMorphScale) * surfaceValue),
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
                                offset: Offset(-_kMorphSlide * surfaceValue, 0),
                                child: Transform.rotate(
                                  angle: (pi / 4) * surfaceValue,
                                  child: Transform.scale(
                                    scale:
                                        1 - ((1 - _kMorphScale) * surfaceValue),
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
      padding: const EdgeInsets.symmetric(vertical: Grid.xxs),
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          _QuickActionItem(
            icon: LucideIcons.hash,
            title: 'Create channel',
            subtitle: 'Start a new stream channel',
            onTap: () => onSelected(_QuickAction.createChannel),
          ),
          _QuickActionItem(
            icon: LucideIcons.messagesSquare,
            title: 'New direct message',
            subtitle: 'Message one or more people',
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
  final String subtitle;
  final VoidCallback onTap;

  const _QuickActionItem({
    required this.icon,
    required this.title,
    required this.subtitle,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final foreground = context.colors.onPrimary;

    return Expanded(
      child: InkWell(
        onTap: onTap,
        child: Padding(
          padding: const EdgeInsets.symmetric(horizontal: Grid.xs),
          child: Row(
            children: [
              Icon(icon, size: 22, color: foreground),
              const SizedBox(width: Grid.twelve),
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
                    const SizedBox(height: Grid.quarter),
                    Text(
                      subtitle,
                      maxLines: 1,
                      overflow: TextOverflow.ellipsis,
                      style: context.textTheme.bodySmall?.copyWith(
                        color: foreground.withValues(alpha: 0.72),
                      ),
                    ),
                  ],
                ),
              ),
            ],
          ),
        ),
      ),
    );
  }
}
