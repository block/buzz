import 'package:flutter/widgets.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// The routes currently showing each channel's page, newest last.
///
/// A share to a conversation that is already open returns to that page, so one
/// composer owns the draft and its attachments.
class OpenChannelRoutes {
  final _routes = <String, List<Route<Object?>>>{};

  /// The newest route showing [channelId] that is still on a navigator.
  Route<Object?>? latest(String channelId) {
    final routes = _routes[channelId];
    if (routes == null) return null;
    for (final route in routes.reversed) {
      if (route.isActive) return route;
    }
    return null;
  }

  VoidCallback _register(String channelId, Route<Object?> route) {
    final routes = _routes.putIfAbsent(channelId, () => [])..add(route);
    return () {
      routes.remove(route);
      if (routes.isEmpty) _routes.remove(channelId);
    };
  }
}

/// Returns to [route] from [top]'s screen, as pressing Back would.
///
/// Walks out from [route]'s navigator through each navigator hosting it, then
/// pops what covers each host, outermost first. It stops at a page that
/// refuses to close, leaving [route] covered rather than discarding that
/// page's work. Returns false, popping nothing, when [route] isn't shown
/// under [top].
bool bringOpenChannelForward(Route<Object?> route, NavigatorState top) {
  final targets = <Route<Object?>>[];
  Route<Object?>? target = route;
  while (target != null) {
    final navigator = target.navigator;
    if (navigator == null) return false;
    targets.add(target);
    if (navigator == top) break;
    target = ModalRoute.of(navigator.context);
  }
  if (target == null) return false;
  for (final target in targets.reversed) {
    var refused = false;
    target.navigator!.popUntil((covering) {
      if (covering == target) return true;
      refused = covering.popDisposition == RoutePopDisposition.doNotPop;
      return refused;
    });
    if (refused) break;
  }
  return true;
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
