import 'package:flutter/widgets.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// The routes currently showing each channel's page, newest last.
///
/// A share to a conversation that is already open goes to that page's
/// composer, so one composer owns the draft and its attachments.
class OpenChannelRoutes {
  final _routes = <String, List<Route<Object?>>>{};

  /// Channel and thread routes in the order they mounted, oldest first.
  final _mounted = <Route<Object?>>[];
  final _threads = <Route<Object?>>{};

  /// The newest active route showing [channelId], optionally on [navigator].
  Route<Object?>? latest(String channelId, {NavigatorState? navigator}) {
    final routes = _routes[channelId];
    if (routes == null) return null;
    for (final route in routes.reversed) {
      if (route.isActive &&
          (navigator == null || route.navigator == navigator)) {
        return route;
      }
    }
    return null;
  }

  /// Whether a thread page other than [current] mounted above [open] on its
  /// navigator. Returning to [open] would dispose that thread and its
  /// composer's unsent attachments.
  bool coversThread(Route<Object?> open, Route<Object?> current) {
    final index = _mounted.indexOf(open);
    assert(index >= 0, 'coversThread needs a registered channel route');
    return _mounted
        .skip(index + 1)
        .any(
          (route) =>
              route != current &&
              _threads.contains(route) &&
              route.isActive &&
              route.navigator == open.navigator,
        );
  }

  VoidCallback _register(String channelId, Route<Object?> route) {
    final routes = _routes.putIfAbsent(channelId, () => [])..add(route);
    _mounted.add(route);
    return () {
      routes.remove(route);
      if (routes.isEmpty) _routes.remove(channelId);
      _mounted.remove(route);
    };
  }

  VoidCallback _registerThread(Route<Object?> route) {
    _mounted.add(route);
    _threads.add(route);
    return () {
      _mounted.remove(route);
      _threads.remove(route);
    };
  }
}

/// Whether [route] is what [top] shows: current on its own navigator and on
/// every navigator hosting it, up to [top].
bool isOpenChannelShown(Route<Object?> route, NavigatorState top) {
  Route<Object?>? shown = route;
  while (shown != null && shown.isCurrent) {
    final navigator = shown.navigator;
    if (navigator == top) return true;
    if (navigator == null) return false;
    shown = ModalRoute.of(navigator.context);
  }
  return false;
}

final openChannelRoutesProvider = Provider<OpenChannelRoutes>(
  (ref) => OpenChannelRoutes(),
);

/// Records that this page's route shows [channelId] while it is mounted.
void useOpenChannelRoute(WidgetRef ref, String channelId) {
  final route = ModalRoute.of(useContext());
  useEffect(() {
    if (route == null) return null;
    return ref.read(openChannelRoutesProvider)._register(channelId, route);
  }, [channelId, route]);
}

/// Records that this page's route shows a thread while it is mounted.
void useOpenThreadRoute(WidgetRef ref) {
  final route = ModalRoute.of(useContext());
  useEffect(() {
    if (route == null) return null;
    return ref.read(openChannelRoutesProvider)._registerThread(route);
  }, [route]);
}
