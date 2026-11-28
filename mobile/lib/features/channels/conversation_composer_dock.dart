import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/ios_navigation_metrics.dart';
import 'composer_dock_size_reporter.dart';

/// The fold across the window, in window coordinates, while the device is
/// half open like a laptop.
Rect? laptopDivisionOf(BuildContext context) {
  final geometry = FoldGeometry.of(context);
  final division = geometry.division;
  if (division != null &&
      division.axis == Axis.horizontal &&
      division.isHalfOpened) {
    return division.bounds;
  }
  final view = View.of(context);
  return IosNavigationMetrics.of(
    context,
  ).laptopDivisionFor(view.physicalSize / view.devicePixelRatio);
}

/// Floats a conversation's composer over its timeline.
///
/// The composer sits at the bottom, and [onHeightChanged] reports its height,
/// which the timeline uses as scroll padding. While the device is half open
/// like a laptop, the messages stay above the fold and the composer sits right
/// below it, over a plain lower half. When the keyboard fills the lower half,
/// the composer moves right above the fold instead. The reported height then
/// reaches up to the fold or the composer, whichever is higher.
class ConversationComposerDock extends HookWidget {
  const ConversationComposerDock({
    super.key,
    required this.onHeightChanged,
    required this.child,
    this.composerKey,
  });

  final ValueChanged<double> onHeightChanged;
  final Widget child;

  /// Identifies the composer itself, whose bounds tests and callers measure.
  final Key? composerKey;

  @override
  Widget build(BuildContext context) {
    final fold = laptopDivisionOf(context);
    final composerHeight = useState<double?>(null);
    final origin = useState<Offset?>(null);
    final reported = useRef<double?>(null);
    final dockKey = useMemoized(GlobalKey.new);

    void report(double height) {
      final previous = reported.value;
      if (previous != null && (previous - height).abs() < 0.5) return;
      reported.value = height;
      onHeightChanged(height);
    }

    return LayoutBuilder(
      key: dockKey,
      builder: (context, constraints) {
        final size = constraints.biggest;
        final height = composerHeight.value;
        double? foldTop;
        double? foldBottom;
        if (fold == null) {
          // Forget where the dock sat, so a fold that comes back after the
          // dock moved is never placed against the old position.
          if (origin.value != null) {
            WidgetsBinding.instance.addPostFrameCallback((_) {
              if (context.mounted) origin.value = null;
            });
          }
        } else {
          // The fold is in window coordinates; measure where the dock sits.
          WidgetsBinding.instance.addPostFrameCallback((_) {
            if (!context.mounted) return;
            final box = dockKey.currentContext?.findRenderObject();
            if (box is! RenderBox || !box.hasSize) return;
            final measured = box.localToGlobal(Offset.zero);
            if (origin.value != measured) origin.value = measured;
          });
          final top = origin.value?.dy;
          if (top != null) {
            foldTop = fold.top - top;
            foldBottom = fold.bottom - top;
          }
        }
        final split =
            foldTop != null &&
            foldBottom != null &&
            height != null &&
            size.isFinite &&
            foldTop > 0 &&
            foldBottom < size.height;
        // Below the fold, or right above it once the keyboard fills the
        // lower half, so the composer never covers the hinge.
        final composerTop = !split
            ? null
            : foldBottom + height <= size.height
            ? foldBottom
            : foldTop - height >= 0
            ? foldTop - height
            : size.height - height;
        final panelTop = composerTop == null
            ? size.height
            : composerTop < foldTop!
            ? composerTop
            : foldTop;
        if (height != null) {
          WidgetsBinding.instance.addPostFrameCallback(
            (_) => report(split ? size.height - panelTop : height),
          );
        }
        // One structure for both layouts keeps the composer's draft and focus
        // when the hinge moves.
        return Stack(
          fit: StackFit.expand,
          children: [
            Positioned(
              key: const ValueKey('conversation-laptop-panel'),
              left: 0,
              right: 0,
              top: panelTop,
              bottom: 0,
              child: ColoredBox(color: context.colors.surface),
            ),
            Positioned(
              left: 0,
              right: 0,
              top: composerTop,
              bottom: composerTop == null ? 0 : null,
              child: Align(
                alignment: composerTop == null
                    ? Alignment.bottomCenter
                    : Alignment.topCenter,
                child: ComposerDockSizeReporter(
                  key: composerKey,
                  onHeightChanged: (value) {
                    composerHeight.value = value;
                    // Outside the laptop pose, report in the same frame as
                    // the composer measured itself.
                    if (!split) report(value);
                  },
                  child: child,
                ),
              ),
            ),
          ],
        );
      },
    );
  }
}
