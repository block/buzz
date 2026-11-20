import 'package:flutter/widgets.dart';

import 'diagnostic_reporter.dart';
import 'diagnostic_screens.dart';

/// Leaves a breadcrumb for each screen the person moves to or back to, so a
/// report shows which screens they were on.
///
/// Attach one to each navigator that holds screens: the app's root navigator
/// and the pane navigator that holds an open conversation. Only pages
/// ([PageRoute]) count. Dialogs, sheets and menus are popup routes with no
/// screen of their own, so opening or closing one leaves nothing. A build
/// without a reporting destination does nothing, not even looking at routes.
class DiagnosticNavigationObserver extends NavigatorObserver {
  DiagnosticNavigationObserver(this._reporter);

  final DiagnosticReporter _reporter;

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      _arrived(route);

  @override
  void didReplace({Route<dynamic>? newRoute, Route<dynamic>? oldRoute}) =>
      _arrived(newRoute);

  @override
  void didPop(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      _revealed(route, previousRoute);

  @override
  void didRemove(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      _revealed(route, previousRoute);

  void _arrived(Route<dynamic>? route) {
    if (route is PageRoute<dynamic>) _announce(route);
  }

  void _revealed(Route<dynamic> left, Route<dynamic>? revealed) {
    if (left is PageRoute<dynamic> && revealed is PageRoute<dynamic>) {
      _announce(revealed);
    }
  }

  /// Names [route] once the frame has built it, if it is then what the person
  /// sees. Several pops in one go, or a route replaced at once, leave only the
  /// screen they end on.
  void _announce(PageRoute<dynamic> route) {
    if (!_reporter.reporting) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!_shown(route)) return;
      _reporter.leaveBreadcrumb(
        DiagnosticBreadcrumb.screen(DiagnosticScreens.of(route)),
      );
    });
  }

  /// Whether [route] is current on its navigator and every navigator hosting
  /// it.
  static bool _shown(Route<dynamic> route) {
    if (!route.isCurrent) return false;
    final navigator = route.navigator;
    if (navigator == null) return false;
    final host = ModalRoute.of(navigator.context);
    return host == null || _shown(host);
  }
}
