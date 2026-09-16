part of '../message_actions.dart';

const _reactionTrayMaxWidth = (52.0 * 6) + (Grid.twelve * 5) + (Grid.xxs * 2);
const _reactionTrayMaxHeight = 52.0 + (Grid.xxs * 2);
const _reactionTraySpringAllowance = 16.0;
const _reactionPopoverDuration = Duration(milliseconds: 320);
final _reactionSpringCurve = _SpringEaseOutCurve(
  duration: const Duration(milliseconds: 260),
  bounce: 0.22,
);

/// Presents the frequently-used reaction tray over a long-pressed message.
///
/// The conversation is frosted around [anchorRect] so the selected message
/// remains visually connected to the tray.
void _showMessageReactionPopover({
  required BuildContext context,
  required WidgetRef ref,
  required TimelineMessage message,
  required Rect anchorRect,
  required EdgeInsets spotlightPadding,
}) {
  unawaited(
    _presentMessageReactionPopover(
      context: context,
      ref: ref,
      message: message,
      anchorRect: anchorRect,
      spotlightPadding: spotlightPadding,
    ),
  );
}

Future<void> _presentMessageReactionPopover({
  required BuildContext context,
  required WidgetRef ref,
  required TimelineMessage message,
  required Rect anchorRect,
  required EdgeInsets spotlightPadding,
}) async {
  if (_messageActionPresentationInFlight) return;
  _messageActionPresentationInFlight = true;

  try {
    unawaited(HapticFeedback.mediumImpact());
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final dialogRoute = RawDialogRoute<void>(
      barrierDismissible: true,
      barrierLabel: 'Dismiss reaction picker',
      barrierColor: Colors.transparent,
      transitionDuration: reduceMotion
          ? Duration.zero
          : _reactionPopoverDuration,
      transitionBuilder: (context, animation, secondaryAnimation, child) =>
          child,
      pageBuilder: (dialogContext, animation, secondaryAnimation) =>
          _MessageReactionPopover(
            anchorRect: anchorRect,
            spotlightPadding: spotlightPadding,
            animation: animation,
            message: message,
            pageContext: context,
            pageRef: ref,
          ),
    );
    var routePushed = false;
    messageActionBackdropActive.value = true;
    try {
      // Remove UIKit glass views from Flutter's platform-view overlay before the
      // backdrop filter paints, otherwise they leave sharp rectangular holes.
      await WidgetsBinding.instance.endOfFrame;
      if (!context.mounted) return;
      final popResult = Navigator.of(
        context,
        rootNavigator: true,
      ).push(dialogRoute);
      routePushed = true;
      await popResult;
    } finally {
      if (routePushed) await dialogRoute.completed;
      messageActionBackdropActive.value = false;
    }
  } finally {
    _messageActionPresentationInFlight = false;
  }
}

class _MessageReactionPopover extends HookWidget {
  final Rect anchorRect;
  final EdgeInsets spotlightPadding;
  final Animation<double> animation;
  final TimelineMessage message;
  final BuildContext pageContext;
  final WidgetRef pageRef;

  const _MessageReactionPopover({
    required this.anchorRect,
    required this.spotlightPadding,
    required this.animation,
    required this.message,
    required this.pageContext,
    required this.pageRef,
  });

  @override
  Widget build(BuildContext context) {
    final mediaQuery = MediaQuery.of(context);
    final selectionStarted = useRef(false);

    void select(Object? result, [VoidCallback? effect]) {
      if (selectionStarted.value) return;
      selectionStarted.value = true;
      Navigator.of(context).pop(result);
      effect?.call();
    }

    return LayoutBuilder(
      builder: (context, constraints) {
        final safeTop = mediaQuery.padding.top + mediaQuery.viewInsets.top;
        final safeBottom =
            constraints.maxHeight -
            mediaQuery.padding.bottom -
            mediaQuery.viewInsets.bottom;
        final availableAbove = anchorRect.top - Grid.xxs - safeTop;
        final availableBelow = safeBottom - anchorRect.bottom - Grid.xxs;
        final showAbove =
            availableAbove >= _reactionTrayMaxHeight ||
            (availableBelow < _reactionTrayMaxHeight &&
                availableAbove >= availableBelow);
        final proposedTop = showAbove
            ? anchorRect.top - _reactionTrayMaxHeight - Grid.xxs
            : anchorRect.bottom + Grid.xxs;
        final maxTop = math.max(safeTop, safeBottom - _reactionTrayMaxHeight);
        final top = proposedTop.clamp(safeTop, maxTop).toDouble();
        final trayScaleAlignment = showAbove
            ? Alignment.bottomLeft
            : Alignment.topLeft;
        final trayWidth = math.min(
          _reactionTrayMaxWidth,
          constraints.maxWidth - (Grid.xxs * 2),
        );
        final maxLeft = constraints.maxWidth - trayWidth - Grid.xxs;
        final left = (anchorRect.center.dx - (trayWidth / 2))
            .clamp(Grid.xxs, maxLeft)
            .toDouble();

        return Stack(
          children: [
            Positioned.fill(
              child: ClipPath(
                key: const ValueKey('reaction-popover-background'),
                clipper: _OutsideAnchorClipper(anchorRect, spotlightPadding),
                child: BackdropFilter(
                  key: const ValueKey('reaction-popover-backdrop-filter'),
                  filter: _messageActionBackdropFilter,
                  child: AnimatedBuilder(
                    animation: animation,
                    builder: (context, child) {
                      final opacity = const Interval(
                        0,
                        0.30,
                        curve: Curves.easeOutCubic,
                      ).transform(animation.value);
                      return ColoredBox(
                        key: const ValueKey('reaction-popover-background-tint'),
                        color: context.colors.inverseSurface.withValues(
                          alpha: _messageActionBackdropTintOpacity * opacity,
                        ),
                      );
                    },
                  ),
                ),
              ),
            ),
            Positioned.fill(
              child: GestureDetector(
                behavior: HitTestBehavior.opaque,
                onTap: () => select(null),
              ),
            ),
            Positioned(
              top: top,
              left: left,
              width: trayWidth + _reactionTraySpringAllowance,
              height: _reactionTrayMaxHeight,
              child: _AnimatedReactionTray(
                trayKey: const ValueKey('reaction-popover-tray'),
                animation: animation,
                trayWidth: trayWidth,
                scaleAlignment: trayScaleAlignment,
                message: message,
                pageContext: pageContext,
                pageRef: pageRef,
                onSelected: (result, effect) => select(result, effect),
              ),
            ),
          ],
        );
      },
    );
  }
}

class _AnimatedReactionTray extends StatelessWidget {
  final Key trayKey;
  final Animation<double> animation;
  final double trayWidth;
  final AlignmentGeometry scaleAlignment;
  final TimelineMessage message;
  final BuildContext pageContext;
  final WidgetRef pageRef;
  final Object? popResult;
  final void Function(Object? result, VoidCallback effect)? onSelected;

  const _AnimatedReactionTray({
    required this.trayKey,
    required this.animation,
    required this.trayWidth,
    required this.scaleAlignment,
    required this.message,
    required this.pageContext,
    required this.pageRef,
    this.popResult,
    this.onSelected,
  });

  @override
  Widget build(BuildContext context) {
    return AnimatedBuilder(
      animation: animation,
      child: SizedBox(
        width: trayWidth,
        height: _reactionTrayMaxHeight,
        child: Padding(
          padding: const EdgeInsets.all(Grid.xxs),
          child: _QuickReactionRow(
            message: message,
            sheetContext: context,
            pageContext: pageContext,
            pageRef: pageRef,
            presentationAnimation: animation,
            popResult: popResult,
            onSelected: onSelected,
          ),
        ),
      ),
      builder: (context, child) {
        final appearance = const Interval(
          0.04,
          0.23,
          curve: Curves.easeOutCubic,
        ).transform(animation.value);
        final expansion = const Interval(0.16, 0.92).transform(animation.value);
        final springExpansion = _reactionSpringCurve.transform(expansion);
        final width = lerpDouble(
          _reactionTrayMaxHeight,
          trayWidth,
          springExpansion,
        )!;

        return Opacity(
          opacity: appearance,
          child: Transform.scale(
            alignment: scaleAlignment,
            scale: lerpDouble(0.95, 1, appearance)!,
            child: Align(
              alignment: Alignment.centerLeft,
              child: SizedBox(
                width: width,
                height: _reactionTrayMaxHeight,
                child: Material(
                  key: trayKey,
                  color: context.colors.surface,
                  surfaceTintColor: Colors.transparent,
                  elevation: 8,
                  shadowColor: Colors.black.withValues(alpha: 0.2),
                  shape: const StadiumBorder(),
                  clipBehavior: Clip.antiAlias,
                  child: OverflowBox(
                    alignment: Alignment.centerLeft,
                    minWidth: trayWidth,
                    maxWidth: trayWidth,
                    minHeight: _reactionTrayMaxHeight,
                    maxHeight: _reactionTrayMaxHeight,
                    child: child,
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

class _OutsideAnchorClipper extends CustomClipper<Path> {
  final Rect anchorRect;
  final EdgeInsets spotlightPadding;

  const _OutsideAnchorClipper(this.anchorRect, this.spotlightPadding);

  @override
  Path getClip(Size size) {
    final spotlightRect = Rect.fromLTRB(
      anchorRect.left - spotlightPadding.left,
      anchorRect.top - spotlightPadding.top,
      anchorRect.right + spotlightPadding.right,
      anchorRect.bottom + spotlightPadding.bottom,
    );
    return Path()
      ..fillType = PathFillType.evenOdd
      ..addRect(Offset.zero & size)
      ..addRRect(
        RRect.fromRectAndRadius(spotlightRect, const Radius.circular(Radii.lg)),
      );
  }

  @override
  bool shouldReclip(_OutsideAnchorClipper oldClipper) =>
      oldClipper.anchorRect != anchorRect ||
      oldClipper.spotlightPadding != spotlightPadding;
}

class _SpringEaseOutCurve extends Curve {
  final double _durationSeconds;
  final SpringSimulation _simulation;

  _SpringEaseOutCurve({required Duration duration, required double bounce})
    : _durationSeconds =
          duration.inMicroseconds / Duration.microsecondsPerSecond,
      _simulation = SpringSimulation(
        SpringDescription.withDurationAndBounce(
          duration: duration,
          bounce: bounce,
        ),
        0,
        1,
        0,
        snapToEnd: true,
      );

  @override
  double transformInternal(double t) => _simulation.x(t * _durationSeconds);
}
