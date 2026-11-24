import 'package:flutter/widgets.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

/// The routes currently showing each channel's page, newest last.
///
/// A share to a conversation that is already open goes to that page's
/// composer, so one composer owns the draft and its attachments.
class OpenChannelRoutes {
  final _routes = <String, List<Route<Object?>>>{};
  final _reveals = Expando<void Function(String messageId)>();

  /// The newest route showing [channelId] that is still on a navigator.
  Route<Object?>? latest(String channelId) {
    final routes = _routes[channelId];
    if (routes == null) return null;
    for (final route in routes.reversed) {
      if (route.isActive) return route;
    }
    return null;
  }

  /// Whether [route] is a page showing [channelId].
  bool shows(String channelId, Route<Object?> route) =>
      _routes[channelId]?.contains(route) ?? false;

  /// Asks the page on [route] to bring [messageId] into view once it is shown
  /// again. Does nothing for a page with no timeline to move.
  void reveal(Route<Object?> route, String messageId) =>
      _reveals[route]?.call(messageId);

  VoidCallback _register(String channelId, Route<Object?> route) {
    final routes = _routes.putIfAbsent(channelId, () => [])..add(route);
    return () {
      routes.remove(route);
      if (routes.isEmpty) _routes.remove(channelId);
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

/// Lets a page above this one's route ask its timeline for a message, through
/// [OpenChannelRoutes.reveal].
void useOpenChannelReveal(
  WidgetRef ref,
  void Function(String messageId) reveal,
) {
  final route = ModalRoute.of(useContext());
  final latest = useRef(reveal)..value = reveal;
  useEffect(() {
    if (route == null) return null;
    final routes = ref.read(openChannelRoutesProvider);
    void forward(String messageId) => latest.value(messageId);
    routes._reveals[route] = forward;
    return () {
      if (routes._reveals[route] == forward) routes._reveals[route] = null;
    };
  }, [route]);
}
