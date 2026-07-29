part of '../activity_page.dart';

const _activityPopoverEnterDuration = Duration(milliseconds: 150);
const _activityPopoverExitDuration = Duration(milliseconds: 110);
const _activityPopoverStartScale = 0.96;

enum _ActivityPopoverAlignment { start, end }

Future<T?> _showActivityPopover<T>({
  required BuildContext context,
  required List<PopupMenuEntry<T>> items,
  required double width,
  required _ActivityPopoverAlignment alignment,
  required Color color,
  required ShapeBorder shape,
  required double elevation,
  required Color shadowColor,
  Offset offset = Offset.zero,
  EdgeInsetsGeometry menuPadding = EdgeInsets.zero,
  Clip clipBehavior = Clip.antiAlias,
  Key? surfaceKey,
}) {
  final navigator = Navigator.of(context);
  final overlay = navigator.overlay;
  final triggerRenderObject = context.findRenderObject();
  final overlayRenderObject = overlay?.context.findRenderObject();
  if (triggerRenderObject is! RenderBox || overlayRenderObject is! RenderBox) {
    return Future<T?>.value();
  }

  final triggerRect = MatrixUtils.transformRect(
    triggerRenderObject.getTransformTo(overlayRenderObject),
    Offset.zero & triggerRenderObject.size,
  );
  final overlayRect = Offset.zero & overlayRenderObject.size;
  final mediaQuery = MediaQuery.of(context);

  return navigator.push<T>(
    _ActivityPopoverRoute<T>(
      position: RelativeRect.fromRect(triggerRect, overlayRect),
      items: items,
      width: width,
      alignment: alignment,
      offset: offset,
      color: color,
      shape: shape,
      elevation: elevation,
      shadowColor: shadowColor,
      menuPadding: menuPadding,
      clipBehavior: clipBehavior,
      surfaceKey: surfaceKey,
      screenPadding: EdgeInsets.fromLTRB(
        math.max(Grid.xxs, mediaQuery.padding.left),
        math.max(Grid.xxs, mediaQuery.padding.top),
        math.max(Grid.xxs, mediaQuery.padding.right),
        math.max(Grid.xxs, mediaQuery.padding.bottom),
      ),
      reducedMotion: mediaQuery.disableAnimations,
      barrierLabel: MaterialLocalizations.of(context).modalBarrierDismissLabel,
    ),
  );
}

class _ActivityPopoverRoute<T> extends PopupRoute<T> {
  final RelativeRect position;
  final List<PopupMenuEntry<T>> items;
  final double width;
  final _ActivityPopoverAlignment alignment;
  final Offset offset;
  final Color color;
  final ShapeBorder shape;
  final double elevation;
  final Color shadowColor;
  final EdgeInsetsGeometry menuPadding;
  final Clip clipBehavior;
  final Key? surfaceKey;
  final EdgeInsets screenPadding;
  final bool reducedMotion;
  final String _barrierLabel;

  _ActivityPopoverRoute({
    required this.position,
    required this.items,
    required this.width,
    required this.alignment,
    required this.offset,
    required this.color,
    required this.shape,
    required this.elevation,
    required this.shadowColor,
    required this.menuPadding,
    required this.clipBehavior,
    required this.surfaceKey,
    required this.screenPadding,
    required this.reducedMotion,
    required String barrierLabel,
  }) : _barrierLabel = barrierLabel;

  @override
  Color? get barrierColor => null;

  @override
  bool get barrierDismissible => true;

  @override
  String? get barrierLabel => _barrierLabel;

  @override
  Duration get transitionDuration =>
      reducedMotion ? Duration.zero : _activityPopoverEnterDuration;

  @override
  Duration get reverseTransitionDuration =>
      reducedMotion ? Duration.zero : _activityPopoverExitDuration;

  @override
  Widget buildPage(
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
  ) {
    final curvedAnimation = animation.drive(
      CurveTween(curve: Curves.easeOutCubic),
    );
    final scaleAnimation = Tween<double>(
      begin: _activityPopoverStartScale,
      end: 1,
    ).animate(curvedAnimation);
    final transformOrigin = switch (alignment) {
      _ActivityPopoverAlignment.start => Alignment.topLeft,
      _ActivityPopoverAlignment.end => Alignment.topRight,
    };

    return CustomSingleChildLayout(
      delegate: _ActivityPopoverLayoutDelegate(
        position: position,
        alignment: alignment,
        offset: offset,
        screenPadding: screenPadding,
      ),
      child: FadeTransition(
        key: const ValueKey('activity-popover-fade'),
        opacity: curvedAnimation,
        child: ScaleTransition(
          key: const ValueKey('activity-popover-scale'),
          scale: scaleAnimation,
          alignment: transformOrigin,
          child: Material(
            key: surfaceKey,
            type: MaterialType.card,
            color: color,
            surfaceTintColor: Colors.transparent,
            elevation: elevation,
            shadowColor: shadowColor,
            shape: shape,
            clipBehavior: clipBehavior,
            child: SizedBox(
              width: width,
              child: Semantics(
                role: SemanticsRole.menu,
                scopesRoute: true,
                namesRoute: true,
                explicitChildNodes: true,
                child: SingleChildScrollView(
                  padding: menuPadding,
                  child: ListBody(children: items),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _ActivityPopoverLayoutDelegate extends SingleChildLayoutDelegate {
  final RelativeRect position;
  final _ActivityPopoverAlignment alignment;
  final Offset offset;
  final EdgeInsets screenPadding;

  const _ActivityPopoverLayoutDelegate({
    required this.position,
    required this.alignment,
    required this.offset,
    required this.screenPadding,
  });

  @override
  BoxConstraints getConstraintsForChild(BoxConstraints constraints) {
    return BoxConstraints.loose(
      Size(
        constraints.maxWidth - screenPadding.horizontal,
        constraints.maxHeight - screenPadding.vertical,
      ),
    );
  }

  @override
  Offset getPositionForChild(Size size, Size childSize) {
    final anchorBottom = size.height - position.bottom;
    final desiredX = switch (alignment) {
      _ActivityPopoverAlignment.start => position.left + offset.dx,
      _ActivityPopoverAlignment.end =>
        size.width - position.right - childSize.width + offset.dx,
    };
    final minX = screenPadding.left;
    final maxX = size.width - screenPadding.right - childSize.width;
    final x = desiredX.clamp(minX, maxX).toDouble();

    final belowY = anchorBottom + offset.dy;
    final aboveY = position.top - childSize.height - offset.dy;
    final maxY = size.height - screenPadding.bottom - childSize.height;
    final desiredY =
        belowY + childSize.height <= size.height - screenPadding.bottom
        ? belowY
        : aboveY;
    final y = desiredY.clamp(screenPadding.top, maxY).toDouble();

    return Offset(x, y);
  }

  @override
  bool shouldRelayout(_ActivityPopoverLayoutDelegate oldDelegate) {
    return position != oldDelegate.position ||
        alignment != oldDelegate.alignment ||
        offset != oldDelegate.offset ||
        screenPadding != oldDelegate.screenPadding;
  }
}
