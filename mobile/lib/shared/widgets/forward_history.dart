import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Root-navigator forward history, like a browser's Forward button.
///
/// Popping a [MaterialPageRoute] remembers how to rebuild it, plus whatever
/// state its page saved with [useForwardRestoration]. Pushing any other page
/// discards that history, so Forward only retraces steps the user just
/// backed out of.
final forwardHistory = ForwardHistory();

typedef _PoppedPage = ({
  WidgetBuilder builder,
  RouteSettings settings,
  bool fullscreenDialog,
  Object? state,
});

class ForwardHistory extends NavigatorObserver {
  // Only what is needed to rebuild a page, not the disposed route itself.
  final _popped = <_PoppedPage>[];
  final _restoring = <Route<dynamic>>{};
  final _savers = <Route<dynamic>, Object? Function()>{};
  final _restoredStates = <Route<dynamic>, Object>{};

  /// Whether there is a page to go forward to.
  final canGoForward = ValueNotifier(false);

  /// Pushes the most recently popped page and leaves its transition at the
  /// start, for a gesture to drive. Returns null when there is nothing to
  /// restore.
  ForwardGesture? beginGesture() {
    final navigator = this.navigator;
    if (navigator == null || _popped.isEmpty) return null;
    final popped = _popped.removeLast();
    final route = _ForwardPageRoute(
      builder: popped.builder,
      settings: popped.settings,
      fullscreenDialog: popped.fullscreenDialog,
    );
    if (popped.state case final state?) _restoredStates[route] = state;
    _restoring.add(route);
    _sync();
    navigator.push(route);
    navigator.didStartUserGesture();
    // Pushing started the transition; hand it to the finger instead.
    route.transition.value = 0;
    return ForwardGesture._(navigator, route);
  }

  /// Forgets every remembered page, e.g. when switching communities.
  void clear() {
    _popped.clear();
    _sync();
  }

  @override
  void didPop(Route<dynamic> route, Route<dynamic>? previousRoute) {
    _restoredStates.remove(route);
    if (route is MaterialPageRoute<dynamic>) {
      _popped.add((
        builder: route.builder,
        settings: route.settings,
        fullscreenDialog: route.fullscreenDialog,
        state: _savers[route]?.call(),
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
  void didRemove(Route<dynamic> route, Route<dynamic>? previousRoute) {
    _restoredStates.remove(route);
    clear();
  }

  @override
  void didReplace({Route<dynamic>? newRoute, Route<dynamic>? oldRoute}) =>
      clear();

  void _sync() => canGoForward.value = _popped.isNotEmpty;
}

/// An in-progress forward swipe over a just-pushed page.
class ForwardGesture {
  ForwardGesture._(this._navigator, this._route);

  static const _maxSettleDuration = Duration(milliseconds: 350);
  static const _flingVelocity = 1.0;

  final NavigatorState _navigator;
  final _ForwardPageRoute _route;

  AnimationController get _transition => _route.transition;

  /// Moves the incoming page by [delta], a fraction of the screen width.
  void update(double delta) => _transition.value += delta;

  /// Finishes the swipe: completes it when flung forward or more than half
  /// way, otherwise slides the page back off and keeps it in history.
  /// [velocity] is in screen widths per second, positive going forward.
  void end(double velocity) {
    final complete =
        velocity >= _flingVelocity ||
        (velocity > -_flingVelocity && _transition.value > 0.5);
    if (complete) {
      _transition.animateTo(
        1,
        duration: _maxSettleDuration * (1 - _transition.value),
        curve: Curves.fastLinearToSlowEaseIn,
      );
    } else {
      _navigator.pop();
    }
    if (!_transition.isAnimating) {
      _navigator.didStopUserGesture();
      return;
    }
    void settled(AnimationStatus status) {
      if (status.isAnimating) return;
      _transition.removeStatusListener(settled);
      _navigator.didStopUserGesture();
    }

    _transition.addStatusListener(settled);
  }
}

class _ForwardPageRoute extends MaterialPageRoute<void> {
  _ForwardPageRoute({
    required super.builder,
    required super.settings,
    required super.fullscreenDialog,
  });

  AnimationController get transition => controller!;
}

/// Lets a page come back as the user left it after a forward swipe.
///
/// [ForwardRestoration.restored] is the state this page saved when it was
/// last popped, if it is being restored. Set [ForwardRestoration.save] to
/// describe the current state; it runs when the page is popped.
ForwardRestoration<T> useForwardRestoration<T extends Object>() {
  final route = ModalRoute.of(useContext());
  final restoration = useMemoized(
    () => ForwardRestoration<T>._(switch (forwardHistory._restoredStates.remove(
      route,
    )) {
      final T state => state,
      _ => null,
    }),
    [route],
  );
  useEffect(() {
    if (route == null) return null;
    forwardHistory._savers[route] = () => restoration.save?.call();
    return () => forwardHistory._savers.remove(route);
  }, [route, restoration]);
  return restoration;
}

class ForwardRestoration<T extends Object> {
  ForwardRestoration._(this.restored);

  final T? restored;
  T? Function()? save;
}

/// Brings back the last popped page when the user swipes left from the right
/// edge, tracking the finger like the iOS back swipe.
class ForwardSwipeHost extends HookWidget {
  const ForwardSwipeHost({required this.child, super.key});

  final Widget child;

  static const _edgeWidth = 20.0;

  @override
  Widget build(BuildContext context) {
    final canGoForward = useValueListenable(forwardHistory.canGoForward);
    final gesture = useRef<ForwardGesture?>(null);
    // Keeps the edge alive for a swipe that restores the last page.
    final dragging = useState(false);
    final width = MediaQuery.sizeOf(context).width;
    void start() {
      dragging.value = true;
      gesture.value = forwardHistory.beginGesture();
    }

    void end(double velocity) {
      gesture.value?.end(velocity / width);
      gesture.value = null;
      dragging.value = false;
    }

    return Stack(
      fit: StackFit.expand,
      children: [
        child,
        if (canGoForward || dragging.value)
          PositionedDirectional(
            top: 0,
            bottom: 0,
            end: 0,
            width: _edgeWidth + MediaQuery.paddingOf(context).right,
            child: GestureDetector(
              // Opaque: the navigator cancels pointers it saw when the
              // restored page is pushed, which would end this drag.
              behavior: HitTestBehavior.opaque,
              onHorizontalDragStart: (_) => start(),
              onHorizontalDragUpdate: (details) =>
                  gesture.value?.update(-(details.primaryDelta ?? 0) / width),
              onHorizontalDragEnd: (details) =>
                  end(-(details.primaryVelocity ?? 0)),
              onHorizontalDragCancel: () => end(0),
            ),
          ),
      ],
    );
  }
}
