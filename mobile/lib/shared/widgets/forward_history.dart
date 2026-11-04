import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Root-navigator forward history, like a browser's Forward button.
///
/// Popping a [MaterialPageRoute] remembers how to rebuild it. Pushing any
/// other page discards that history, so Forward only retraces steps the user
/// just backed out of.
final forwardHistory = ForwardHistory();

class ForwardHistory extends NavigatorObserver {
  // Only what is needed to rebuild a page, not the disposed route itself.
  final _popped =
      <
        ({WidgetBuilder builder, RouteSettings settings, bool fullscreenDialog})
      >[];
  final _restoring = <Route<dynamic>>{};

  /// Whether [goForward] has a page to restore.
  final canGoForward = ValueNotifier(false);

  /// Pushes a fresh copy of the most recently popped page.
  void goForward() {
    final navigator = this.navigator;
    if (navigator == null || _popped.isEmpty) return;
    final popped = _popped.removeLast();
    final route = MaterialPageRoute<void>(
      builder: popped.builder,
      settings: popped.settings,
      fullscreenDialog: popped.fullscreenDialog,
    );
    _restoring.add(route);
    _sync();
    navigator.push(route);
  }

  /// Forgets every remembered page, e.g. when switching communities.
  void clear() {
    _popped.clear();
    _sync();
  }

  @override
  void didPop(Route<dynamic> route, Route<dynamic>? previousRoute) {
    if (route is MaterialPageRoute<dynamic>) {
      _popped.add((
        builder: route.builder,
        settings: route.settings,
        fullscreenDialog: route.fullscreenDialog,
      ));
      _sync();
    }
  }

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) {
    if (_restoring.remove(route)) return;
    // Sheets, dialogs and menus sit on top of the current page without
    // navigating away from it.
    if (route is PopupRoute<dynamic>) return;
    clear();
  }

  @override
  void didRemove(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      clear();

  @override
  void didReplace({Route<dynamic>? newRoute, Route<dynamic>? oldRoute}) =>
      clear();

  void _sync() => canGoForward.value = _popped.isNotEmpty;
}

/// Restores the last popped page when the user swipes left from the right
/// edge: the mirror image of the iOS back swipe.
class ForwardSwipeHost extends HookWidget {
  const ForwardSwipeHost({required this.child, super.key});

  final Widget child;

  static const _edgeWidth = 20.0;
  static const _minDistance = 60.0;
  static const _minVelocity = 400.0;

  @override
  Widget build(BuildContext context) {
    final canGoForward = useValueListenable(forwardHistory.canGoForward);
    final distance = useRef(0.0);
    void end(DragEndDetails details) {
      final flung = -(details.primaryVelocity ?? 0) >= _minVelocity;
      if (distance.value >= _minDistance || flung) forwardHistory.goForward();
      distance.value = 0;
    }

    return Stack(
      fit: StackFit.expand,
      children: [
        child,
        if (canGoForward)
          PositionedDirectional(
            top: 0,
            bottom: 0,
            end: 0,
            width: _edgeWidth + MediaQuery.paddingOf(context).right,
            child: GestureDetector(
              behavior: HitTestBehavior.translucent,
              onHorizontalDragStart: (_) => distance.value = 0,
              onHorizontalDragUpdate: (details) =>
                  distance.value -= details.primaryDelta ?? 0,
              onHorizontalDragEnd: end,
              onHorizontalDragCancel: () => distance.value = 0,
            ),
          ),
      ],
    );
  }
}
