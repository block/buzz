part of '../animated_avatar_capture.dart';

enum _AnimatedReviewSection { person, color, poster }

class _RepositionablePreviewSemantics extends StatelessWidget {
  const _RepositionablePreviewSemantics({
    required this.offset,
    required this.onMove,
    required this.child,
  });

  final Offset offset;
  final ValueChanged<Offset> onMove;
  final Widget child;

  @override
  Widget build(BuildContext context) => Semantics(
    label: 'Avatar position',
    value:
        '${(offset.dx * 100).round()} horizontal, '
        '${(offset.dy * 100).round()} vertical',
    customSemanticsActions: {
      if (offset.dx > -1)
        const CustomSemanticsAction(label: 'Move left'): () =>
            onMove(const Offset(-0.1, 0)),
      if (offset.dx < 1)
        const CustomSemanticsAction(label: 'Move right'): () =>
            onMove(const Offset(0.1, 0)),
      if (offset.dy > -1)
        const CustomSemanticsAction(label: 'Move up'): () =>
            onMove(const Offset(0, -0.1)),
      if (offset.dy < 1)
        const CustomSemanticsAction(label: 'Move down'): () =>
            onMove(const Offset(0, 0.1)),
    },
    child: ExcludeSemantics(child: child),
  );
}

class _AnimatedReviewNav extends StatelessWidget {
  const _AnimatedReviewNav({
    required this.selected,
    required this.onSelected,
    required this.onRetake,
  });

  final _AnimatedReviewSection selected;
  final ValueChanged<_AnimatedReviewSection> onSelected;
  final VoidCallback? onRetake;

  @override
  Widget build(BuildContext context) => Row(
    crossAxisAlignment: CrossAxisAlignment.start,
    children: [
      Expanded(
        child: AvatarEditorOptionButton(
          icon: LucideIcons.userRound,
          iosIcon: IosGlassNavigationIcon.person,
          label: 'You',
          selected: selected == _AnimatedReviewSection.person,
          onTap: () => onSelected(_AnimatedReviewSection.person),
        ),
      ),
      const SizedBox(width: Grid.half),
      Expanded(
        child: AvatarEditorOptionButton(
          icon: LucideIcons.palette,
          iosIcon: IosGlassNavigationIcon.palette,
          label: 'Background',
          selected: selected == _AnimatedReviewSection.color,
          onTap: () => onSelected(_AnimatedReviewSection.color),
        ),
      ),
      const SizedBox(width: Grid.half),
      Expanded(
        child: AvatarEditorOptionButton(
          icon: LucideIcons.galleryThumbnails,
          iosIcon: IosGlassNavigationIcon.frame,
          label: 'Frame',
          selected: selected == _AnimatedReviewSection.poster,
          onTap: () => onSelected(_AnimatedReviewSection.poster),
        ),
      ),
      const SizedBox(width: Grid.half),
      Container(width: 1, height: 64, color: context.colors.outlineVariant),
      const SizedBox(width: Grid.half),
      Expanded(
        child: AvatarEditorOptionButton(
          icon: LucideIcons.camera,
          iosIcon: IosGlassNavigationIcon.camera,
          label: 'Retake',
          selected: false,
          onTap: onRetake,
        ),
      ),
    ],
  );
}

class _AnimatedFramingControl extends HookWidget {
  const _AnimatedFramingControl({
    super.key,
    required this.scale,
    required this.outline,
    required this.onScaleChanged,
    required this.onOutlineChanged,
  });

  final double scale;
  final bool outline;
  final ValueChanged<double> onScaleChanged;
  final ValueChanged<bool> onOutlineChanged;

  @override
  Widget build(BuildContext context) {
    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        Row(
          children: [
            Expanded(
              child: _DesktopStyleFramingSlider(
                value: scale,
                onChanged: onScaleChanged,
              ),
            ),
            const SizedBox(width: Grid.xxs),
            DecoratedBox(
              decoration: BoxDecoration(
                color: context.colors.surface,
                borderRadius: BorderRadius.circular(Radii.sm),
                border: Border.all(
                  color: context.colors.onSurface.withValues(alpha: 0.08),
                ),
              ),
              child: IconButton(
                key: const ValueKey('animated-avatar-outline'),
                tooltip: outline ? 'Turn outline off' : 'Turn outline on',
                onPressed: () {
                  unawaited(HapticFeedback.selectionClick());
                  onOutlineChanged(!outline);
                },
                icon: Icon(
                  outline ? LucideIcons.circle : LucideIcons.circleDashed,
                  size: 24,
                ),
                constraints: const BoxConstraints.tightFor(
                  width: 56,
                  height: 56,
                ),
              ),
            ),
          ],
        ),
        const SizedBox(height: Grid.xxs),
        Text(
          'Drag the preview to reposition',
          style: context.textTheme.bodySmall,
        ),
      ],
    );
  }
}

class _DesktopStyleFramingSlider extends HookWidget {
  const _DesktopStyleFramingSlider({
    required this.value,
    required this.onChanged,
  });

  static const _min = 0.7;
  static const _max = 2.0;

  final double value;
  final ValueChanged<double> onChanged;

  @override
  Widget build(BuildContext context) {
    final lastTick = useRef((value * 20).round());

    void update(double dx, double width) {
      if (width <= 0) return;
      final progress = (dx / width).clamp(0.0, 1.0).toDouble();
      final next = _min + progress * (_max - _min);
      final tick = (next * 20).round();
      if (tick != lastTick.value) {
        lastTick.value = tick;
        unawaited(HapticFeedback.selectionClick());
      }
      onChanged(next);
    }

    final progress = ((value - _min) / (_max - _min))
        .clamp(0.0, 1.0)
        .toDouble();
    return Semantics(
      slider: true,
      label: 'Avatar size',
      value: '${(value * 100).round()}%',
      increasedValue: value < _max
          ? '${((value + 0.05).clamp(_min, _max) * 100).round()}%'
          : null,
      decreasedValue: value > _min
          ? '${((value - 0.05).clamp(_min, _max) * 100).round()}%'
          : null,
      onIncrease: value < _max
          ? () => onChanged((value + 0.05).clamp(_min, _max).toDouble())
          : null,
      onDecrease: value > _min
          ? () => onChanged((value - 0.05).clamp(_min, _max).toDouble())
          : null,
      child: SizedBox(
        key: const ValueKey('animated-avatar-scale'),
        height: 56,
        child: LayoutBuilder(
          builder: (context, constraints) {
            final handleX = (constraints.maxWidth * progress)
                .clamp(2.0, constraints.maxWidth - 2)
                .toDouble();
            return GestureDetector(
              behavior: HitTestBehavior.opaque,
              onTapDown: (details) =>
                  update(details.localPosition.dx, constraints.maxWidth),
              onHorizontalDragStart: (details) =>
                  update(details.localPosition.dx, constraints.maxWidth),
              onHorizontalDragUpdate: (details) =>
                  update(details.localPosition.dx, constraints.maxWidth),
              child: Stack(
                clipBehavior: Clip.none,
                alignment: Alignment.centerLeft,
                children: [
                  Positioned(
                    left: 0,
                    right: 0,
                    top: 0,
                    bottom: 0,
                    child: DecoratedBox(
                      decoration: BoxDecoration(
                        color: context.colors.surface,
                        borderRadius: BorderRadius.circular(Radii.sm),
                        border: Border.all(
                          color: context.colors.onSurface.withValues(
                            alpha: 0.08,
                          ),
                        ),
                      ),
                    ),
                  ),
                  Positioned(
                    left: 0,
                    top: 0,
                    bottom: 0,
                    width: handleX,
                    child: DecoratedBox(
                      decoration: BoxDecoration(
                        color: context.colors.onSurface.withValues(alpha: 0.06),
                        borderRadius: BorderRadius.circular(Radii.sm),
                      ),
                    ),
                  ),
                  for (var index = 1; index < 8; index++)
                    Positioned(
                      left: constraints.maxWidth * index / 8 - 2,
                      child: Container(
                        width: 4,
                        height: 4,
                        decoration: BoxDecoration(
                          color: context.colors.onSurface.withValues(
                            alpha: 0.16,
                          ),
                          shape: BoxShape.circle,
                        ),
                      ),
                    ),
                  Positioned(
                    left: handleX - 1.5,
                    top: 0,
                    bottom: 0,
                    child: Container(
                      width: 3,
                      decoration: BoxDecoration(
                        color: context.colors.onSurface,
                        borderRadius: BorderRadius.circular(Radii.full),
                      ),
                    ),
                  ),
                ],
              ),
            );
          },
        ),
      ),
    );
  }
}

class _AnimatedFrameControl extends HookWidget {
  const _AnimatedFrameControl({
    super.key,
    required this.frames,
    required this.selectedIndex,
    required this.onSelected,
  });

  final List<Uint8List> frames;
  final int selectedIndex;
  final ValueChanged<int> onSelected;

  @override
  Widget build(BuildContext context) {
    final lastSelection = useRef(selectedIndex);

    void selectIndex(int index) {
      if (frames.isEmpty) return;
      final next = index.clamp(0, frames.length - 1);
      if (next == lastSelection.value) return;
      lastSelection.value = next;
      unawaited(HapticFeedback.selectionClick());
      onSelected(next);
    }

    void selectAt(double dx, double width) {
      if (frames.isEmpty || width <= 0) return;
      final progress = (dx / width).clamp(0.0, 1.0);
      selectIndex((progress * (frames.length - 1)).round());
    }

    return Column(
      mainAxisSize: MainAxisSize.min,
      children: [
        SizedBox(
          key: const ValueKey('animated-avatar-poster-scrubber'),
          height: 64,
          child: LayoutBuilder(
            builder: (context, constraints) {
              const selectorWidth = 52.0;
              final progress = frames.length < 2
                  ? 0.0
                  : selectedIndex / (frames.length - 1);
              return Semantics(
                slider: true,
                label: 'Choose still frame',
                value: '${selectedIndex + 1} of ${frames.length}',
                increasedValue: selectedIndex < frames.length - 1
                    ? '${selectedIndex + 2} of ${frames.length}'
                    : null,
                decreasedValue: selectedIndex > 0
                    ? '$selectedIndex of ${frames.length}'
                    : null,
                onIncrease: selectedIndex < frames.length - 1
                    ? () => selectIndex(selectedIndex + 1)
                    : null,
                onDecrease: selectedIndex > 0
                    ? () => selectIndex(selectedIndex - 1)
                    : null,
                child: GestureDetector(
                  behavior: HitTestBehavior.opaque,
                  onTapDown: (details) =>
                      selectAt(details.localPosition.dx, constraints.maxWidth),
                  onHorizontalDragStart: (details) =>
                      selectAt(details.localPosition.dx, constraints.maxWidth),
                  onHorizontalDragUpdate: (details) =>
                      selectAt(details.localPosition.dx, constraints.maxWidth),
                  child: Stack(
                    children: [
                      Positioned.fill(
                        child: ClipRRect(
                          borderRadius: BorderRadius.circular(Radii.sm),
                          child: Row(
                            children: [
                              for (final (index, frame) in frames.indexed)
                                Expanded(
                                  child: Image.memory(
                                    frame,
                                    key: ValueKey(
                                      'animated-avatar-poster-$index',
                                    ),
                                    height: 64,
                                    fit: BoxFit.cover,
                                    gaplessPlayback: true,
                                  ),
                                ),
                            ],
                          ),
                        ),
                      ),
                      AnimatedPositioned(
                        duration: const Duration(milliseconds: 75),
                        curve: Curves.easeOut,
                        left: (constraints.maxWidth - selectorWidth) * progress,
                        top: 0,
                        bottom: 0,
                        width: selectorWidth,
                        child: IgnorePointer(
                          child: DecoratedBox(
                            decoration: BoxDecoration(
                              color: Colors.white.withValues(alpha: 0.1),
                              borderRadius: BorderRadius.circular(Radii.sm),
                              border: Border.all(color: Colors.white, width: 3),
                              boxShadow: const [
                                BoxShadow(
                                  color: Color(0x52000000),
                                  blurRadius: 16,
                                  offset: Offset(0, 4),
                                ),
                              ],
                            ),
                          ),
                        ),
                      ),
                    ],
                  ),
                ),
              );
            },
          ),
        ),
        const SizedBox(height: Grid.xxs),
        Text('Choose the still frame', style: context.textTheme.bodySmall),
      ],
    );
  }
}
