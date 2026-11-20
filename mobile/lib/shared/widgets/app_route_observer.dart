import 'package:flutter/widgets.dart';

/// Shared route lifecycle for native navigation and recording cleanup.
final appRouteObserver = AppRouteObserver();

/// Also reports reveals after the covering route has finished animating out.
class AppRouteObserver extends RouteObserver<ModalRoute<dynamic>> {
  /// Creates a navigator-owned bridge to the shared route subscriptions.
  /// Each nested navigator needs its own observer instance; Flutter observers
  /// cannot be attached to more than one navigator at a time.
  NavigatorObserver createNestedObserver() => _NestedAppRouteObserver(this);

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

class _NestedAppRouteObserver extends NavigatorObserver {
  _NestedAppRouteObserver(this.delegate);

  final AppRouteObserver delegate;

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      delegate.didPush(route, previousRoute);

  @override
  void didPop(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      delegate.didPop(route, previousRoute);
}
