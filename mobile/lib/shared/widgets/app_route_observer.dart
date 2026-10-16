import 'package:flutter/widgets.dart';

/// Shared route lifecycle for native navigation and recording cleanup.
final appRouteObserver = AppRouteObserver();

/// Also reports reveals after the covering route has finished animating out.
class AppRouteObserver extends RouteObserver<ModalRoute<dynamic>> {
  final _revealListeners = <ModalRoute<dynamic>, Set<VoidCallback>>{};

  /// Registers a layout refresh for the end of a covering route's transition.
  void addRevealListener(ModalRoute<dynamic> route, VoidCallback listener) {
    (_revealListeners[route] ??= {}).add(listener);
  }

  /// Stops reporting completed reveals to a disposed navigation view.
  void removeRevealListener(ModalRoute<dynamic> route, VoidCallback listener) {
    final listeners = _revealListeners[route];
    listeners?.remove(listener);
    if (listeners?.isEmpty ?? false) _revealListeners.remove(route);
  }

  @override
  void didPop(Route<dynamic> route, Route<dynamic>? previousRoute) {
    super.didPop(route, previousRoute);
    if (route is! TransitionRoute<dynamic> ||
        previousRoute is! ModalRoute<dynamic>) {
      return;
    }
    route.completed.then((_) {
      if (!previousRoute.isCurrent) return;
      final listeners = List<VoidCallback>.of(
        _revealListeners[previousRoute] ?? const {},
      );
      for (final listener in listeners) {
        if (_revealListeners[previousRoute]?.contains(listener) ?? false) {
          listener();
        }
      }
    });
  }
}
