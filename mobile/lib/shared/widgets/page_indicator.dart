import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import 'ios_glass_theme_pagination.dart';

/// Appearance-style pagination with native Liquid Glass on iOS.
class PageIndicator extends StatelessWidget {
  /// Creates a tappable and scrubbable page indicator.
  const PageIndicator({
    super.key,
    required this.semanticLabel,
    this.controlKey,
    this.dotKeyPrefix = 'page-indicator-dot-',
    this.containerHeight = 30,
    required this.count,
    required this.selected,
    required this.animateChanges,
    required this.onSelected,
  });

  /// The content name announced with the current page.
  final String semanticLabel;

  /// Optional key for the interactive control.
  final Key? controlKey;

  /// Key prefix identifying the individual page dots.
  final String dotKeyPrefix;

  /// Height of the pill without changing the dot sizes.
  final double containerHeight;

  /// Total number of pages.
  final int count;

  /// Zero-based current page.
  final int selected;

  /// Whether dot position changes animate.
  final bool animateChanges;

  /// Called when a page is selected by tap, scrub, or accessibility.
  final ValueChanged<int> onSelected;

  @override
  Widget build(BuildContext context) {
    final native = defaultTargetPlatform == TargetPlatform.iOS;
    final control = SizedBox(
      key: controlKey,
      height: 54,
      child: Center(
        child: SizedBox(
          width: 116,
          child: native
              ? IosGlassThemePagination(
                  containerHeight: containerHeight,
                  semanticLabel: semanticLabel,
                  count: count,
                  selected: selected,
                  animateChanges: animateChanges,
                  onSelected: onSelected,
                  activeColor: context.colors.onSurface,
                  inactiveColor: context.colors.onSurfaceVariant.withValues(
                    alpha: 0.32,
                  ),
                )
              : _WindowedPagination(
                  containerHeight: containerHeight,
                  dotKeyPrefix: dotKeyPrefix,
                  count: count,
                  selected: selected,
                  animateChanges: animateChanges,
                  onSelected: onSelected,
                ),
        ),
      ),
    );
    // UIKit owns its adjustable element and bridges it through UiKitView.
    if (native) return control;
    return Semantics(
      label: '$semanticLabel ${selected + 1} of $count',
      slider: true,
      value: '${selected + 1}',
      increasedValue: selected < count - 1 ? '${selected + 2}' : null,
      decreasedValue: selected > 0 ? '$selected' : null,
      onIncrease: selected < count - 1 ? () => onSelected(selected + 1) : null,
      onDecrease: selected > 0 ? () => onSelected(selected - 1) : null,
      child: control,
    );
  }
}

typedef _PaginationScrubGeometry = ({
  int windowStart,
  int visibleCount,
  double width,
  bool isRtl,
});

class _WindowedPagination extends HookWidget {
  const _WindowedPagination({
    required this.containerHeight,
    required this.dotKeyPrefix,
    required this.count,
    required this.selected,
    required this.animateChanges,
    required this.onSelected,
  });

  final String dotKeyPrefix;
  final double containerHeight;

  static const _maximumVisibleDots = 7;
  static const _dotSize = 6.0;
  static const _selectedDotSize = 10.0;
  static const _spacing = 6.0;

  final int count;
  final int selected;
  final bool animateChanges;
  final ValueChanged<int> onSelected;

  @override
  Widget build(BuildContext context) {
    final scrubGeometry = useState<_PaginationScrubGeometry?>(null);
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final duration = reduceMotion || !animateChanges
        ? Duration.zero
        : const Duration(milliseconds: 150);
    final currentVisibleCount = count.clamp(1, _maximumVisibleDots);
    final maximumStart = (count - currentVisibleCount).clamp(0, count);
    final centerSlot = currentVisibleCount ~/ 2;
    return LayoutBuilder(
      builder: (context, constraints) {
        final geometry =
            scrubGeometry.value ??
            (
              windowStart: (selected - centerSlot).clamp(0, maximumStart),
              visibleCount: currentVisibleCount,
              width: constraints.maxWidth,
              isRtl: Directionality.of(context) == TextDirection.rtl,
            );
        final windowStart = geometry.windowStart;
        final visibleCount = geometry.visibleCount;
        final isRtl = geometry.isRtl;
        final windowEnd = windowStart + visibleCount - 1;
        final hasEarlierDots = windowStart > 0;
        final hasLaterDots = windowEnd < count - 1;
        final pitch = _dotSize + _spacing;
        final trackWidth =
            visibleCount * _dotSize + (visibleCount - 1) * _spacing;

        void selectFromPosition(double dx) {
          if (count <= 1) return;
          final firstCenter = (geometry.width - trackWidth) / 2 + _dotSize / 2;
          final slot = ((dx - firstCenter) / pitch).round().clamp(
            0,
            visibleCount - 1,
          );
          final page = windowStart + (isRtl ? visibleCount - 1 - slot : slot);
          onSelected(page);
        }

        return GestureDetector(
          behavior: HitTestBehavior.opaque,
          excludeFromSemantics: true,
          // A tap must not select on pointer-down before a drag can snapshot
          // the window. Otherwise the first drag callback already sees a shift.
          onTapUp: (details) => selectFromPosition(details.localPosition.dx),
          onHorizontalDragStart: (details) {
            scrubGeometry.value = geometry;
            selectFromPosition(details.localPosition.dx);
          },
          onHorizontalDragUpdate: (details) =>
              selectFromPosition(details.localPosition.dx),
          onHorizontalDragEnd: (_) => scrubGeometry.value = null,
          onHorizontalDragCancel: () => scrubGeometry.value = null,
          child: SizedBox(
            height: 54,
            child: Center(
              child: Container(
                height: containerHeight,
                padding: const EdgeInsets.symmetric(horizontal: Grid.twelve),
                decoration: BoxDecoration(
                  color: context.colors.surfaceContainerHighest,
                  borderRadius: BorderRadius.circular(Radii.full),
                ),
                child: LayoutBuilder(
                  builder: (context, constraints) {
                    final trackOrigin =
                        (geometry.width - Grid.twelve * 2 - trackWidth) / 2;
                    return ClipRect(
                      child: Stack(
                        children: [
                          for (var page = 0; page < count; page++)
                            _buildDot(
                              context: context,
                              page: page,
                              slot: isRtl
                                  ? visibleCount - 1 - (page - windowStart)
                                  : page - windowStart,
                              visibleCount: visibleCount,
                              trackOrigin: trackOrigin,
                              pitch: pitch,
                              hasEarlierDots: isRtl
                                  ? hasLaterDots
                                  : hasEarlierDots,
                              hasLaterDots: isRtl
                                  ? hasEarlierDots
                                  : hasLaterDots,
                              duration: duration,
                            ),
                        ],
                      ),
                    );
                  },
                ),
              ),
            ),
          ),
        );
      },
    );
  }

  Widget _buildDot({
    required BuildContext context,
    required int page,
    required int slot,
    required int visibleCount,
    required double trackOrigin,
    required double pitch,
    required bool hasEarlierDots,
    required bool hasLaterDots,
    required Duration duration,
  }) {
    final isVisible = slot >= 0 && slot < visibleCount;
    final diameter = page == selected
        ? _selectedDotSize
        : (hasEarlierDots && slot == 0) ||
              (hasLaterDots && slot == visibleCount - 1)
        ? 2.0
        : (hasEarlierDots && slot == 1) ||
              (hasLaterDots && slot == visibleCount - 2)
        ? 4.0
        : _dotSize;
    final centerX = trackOrigin + _dotSize / 2 + slot * pitch;

    return AnimatedPositioned(
      key: ValueKey('$dotKeyPrefix$page'),
      duration: duration,
      curve: Curves.easeInOutCubic,
      left: centerX - diameter / 2,
      top: (containerHeight - diameter) / 2,
      width: diameter,
      height: diameter,
      child: AnimatedOpacity(
        duration: duration,
        opacity: isVisible ? 1 : 0,
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: page == selected
                ? context.colors.onSurface
                : context.colors.onSurfaceVariant.withValues(alpha: 0.32),
            shape: BoxShape.circle,
          ),
        ),
      ),
    );
  }
}
