import 'package:flutter/scheduler.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Tracks pop-up routes, such as sheets and menus, over one pane's navigator.
///
/// A pane's routes block screen readers only inside the pane's own semantics
/// container, so while one is [active] the pane's neighbours must leave the
/// semantics tree themselves.
class PaneModalRoutes extends NavigatorObserver with ChangeNotifier {
  final _routes = <Route<dynamic>>{};

  /// Whether a pop-up route is open over the pane.
  bool get active => _routes.isNotEmpty;

  void _update(void Function() change) {
    final before = active;
    change();
    if (before == active) return;
    // Routes can change while the navigator builds; notify after the frame.
    if (SchedulerBinding.instance.schedulerPhase ==
        SchedulerPhase.persistentCallbacks) {
      SchedulerBinding.instance.addPostFrameCallback((_) => notifyListeners());
    } else {
      notifyListeners();
    }
  }

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) {
    if (route is PopupRoute) _update(() => _routes.add(route));
  }

  @override
  void didPop(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      _update(() => _routes.remove(route));

  @override
  void didRemove(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      _update(() => _routes.remove(route));

  @override
  void didReplace({Route<dynamic>? newRoute, Route<dynamic>? oldRoute}) =>
      _update(() {
        _routes.remove(oldRoute);
        if (newRoute is PopupRoute) _routes.add(newRoute);
      });
}

/// A [PaneModalRoutes] that lives, and rebuilds the caller, with its hook
/// widget.
PaneModalRoutes usePaneModalRoutes() {
  final routes = useMemoized(PaneModalRoutes.new);
  useEffect(() => routes.dispose, [routes]);
  useListenable(routes);
  return routes;
}
