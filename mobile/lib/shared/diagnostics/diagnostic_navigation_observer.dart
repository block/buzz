import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import 'diagnostic_reporter.dart';

/// Names the screen a page widget is, or null for a widget that is not a
/// listed page. The app supplies its list of pages
/// ([diagnosticScreenClassifierProvider]); an unlisted page is reported as
/// [DiagnosticScreen.other], never by its own type name, which release builds
/// can obfuscate.
typedef DiagnosticScreenClassifier = DiagnosticScreen? Function(Widget page);

/// The app's list of named pages. The root scope overrides it, so the list
/// lives with the app rather than below the features it names.
final diagnosticScreenClassifierProvider = Provider<DiagnosticScreenClassifier>(
  (ref) => throw UnimplementedError('The app supplies its list of screens.'),
);

/// The trail shared by every navigator that holds screens.
final diagnosticScreenTrailProvider = Provider<DiagnosticScreenTrail>(
  (ref) => DiagnosticScreenTrail(
    ref.watch(diagnosticReporterProvider),
    () => ref.read(diagnosticScreenClassifierProvider),
  ),
);

/// Leaves a breadcrumb for each screen the person moves to or back to, so a
/// report shows which screens they were on.
///
/// Give an [observer] to each navigator that holds screens: the app's root
/// navigator and the pane navigator that holds an open conversation. Only
/// pages ([PageRoute]) count. Dialogs, sheets and menus are popup routes or
/// [DiagnosticSheetRoute]s with no screen of their own, so opening or closing
/// one leaves nothing. A screen equal to the last one left is skipped. A build
/// without a reporting destination does nothing, not even looking at routes.
class DiagnosticScreenTrail {
  DiagnosticScreenTrail(this._reporter, this._classifier);

  final DiagnosticReporter _reporter;
  final DiagnosticScreenClassifier Function() _classifier;
  final _observers = <DiagnosticNavigationObserver>{};
  DiagnosticScreen? _last;

  /// How many elements a search below a route may look at. A page sits near
  /// the top of its route; this keeps an unlisted page from costing a walk of
  /// everything it shows.
  static const _searchBudget = 3000;

  /// A new observer for one navigator. [DiagnosticNavigationObserver.dispose]
  /// it with its navigator.
  DiagnosticNavigationObserver observer() {
    final observer = DiagnosticNavigationObserver._(this);
    _observers.add(observer);
    return observer;
  }

  /// Names [route] once the frame has built it, if it is then what the person
  /// sees. Several pops in one go, or a route replaced at once, leave only the
  /// screen they end on.
  void _announce(PageRoute<dynamic> route) {
    if (!_reporter.reporting) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (!_shown(route)) return;
      final screen = screenOf(route);
      if (screen == _last) return;
      _last = screen;
      _reporter.leaveBreadcrumb(DiagnosticBreadcrumb.screen(screen));
    });
  }

  /// The screen [route] shows, once it has been built.
  ///
  /// A route has no name of its own here: the app pushes page widgets without
  /// route settings, and where settings exist they can carry IDs. So a route
  /// that hosts a navigator with a page open past its first, such as a
  /// conversation over a list, shows that page. Otherwise it is named by the
  /// first listed page widget below it, and a route showing none is
  /// [DiagnosticScreen.other]. Nothing is read from a route's settings, its
  /// arguments or what the page shows.
  @visibleForTesting
  DiagnosticScreen screenOf(ModalRoute<dynamic> route) {
    if (_openPage(route) case final ModalRoute<dynamic> page) {
      return screenOf(page);
    }
    if (route.subtreeContext case final BuildContext context) {
      final classify = _classifier();
      var budget = _searchBudget;
      DiagnosticScreen? found;
      void visit(Element element) {
        if (found != null || budget-- <= 0) return;
        found = classify(element.widget);
        if (found == null) element.visitChildElements(visit);
      }

      (context as Element).visitChildElements(visit);
      if (found != null) return found!;
    }
    // The first route of a nested navigator is its container, such as the
    // empty list side of a pane, and shows whatever hosts that navigator.
    // After sign-in this is what names Home.
    if (route.isFirst) {
      final navigator = route.navigator;
      final host = navigator == null ? null : ModalRoute.of(navigator.context);
      if (host != null) return screenOf(host);
    }
    return DiagnosticScreen.other;
  }

  /// The page open past the first in a navigator that [route] hosts.
  ModalRoute<dynamic>? _openPage(ModalRoute<dynamic> route) {
    for (final observer in _observers) {
      final navigator = observer.navigator;
      if (navigator == null || ModalRoute.of(navigator.context) != route) {
        continue;
      }
      final page = observer._currentPage;
      if (page != null && !page.isFirst) return page;
    }
    return null;
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

/// A page route that presents a sheet rather than a screen, so it leaves no
/// breadcrumb opening or closing.
mixin DiagnosticSheetRoute<T> on PageRoute<T> {}

/// Reports one navigator's pages to a [DiagnosticScreenTrail].
class DiagnosticNavigationObserver extends NavigatorObserver {
  DiagnosticNavigationObserver._(this._trail);

  final DiagnosticScreenTrail _trail;

  /// This navigator's screens, bottom first. Kept only while reporting.
  final _pages = <PageRoute<dynamic>>[];

  PageRoute<dynamic>? get _currentPage {
    _pages.removeWhere((page) => !page.isActive);
    for (final page in _pages.reversed) {
      if (page.isCurrent) return page;
    }
    return null;
  }

  /// Stops reporting this navigator.
  void dispose() => _trail._observers.remove(this);

  /// Names this navigator's current page again, once the frame has built it.
  /// Call it when a page's content changes in place with no navigation, such
  /// as the root page moving from the splash screen to onboarding or Home.
  void refresh() {
    if (_currentPage case final page?) _trail._announce(page);
  }

  static bool _isScreen(Route<dynamic>? route) =>
      route is PageRoute<dynamic> && route is! DiagnosticSheetRoute<dynamic>;

  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      _arrived(route);

  @override
  void didReplace({Route<dynamic>? newRoute, Route<dynamic>? oldRoute}) {
    _pages.remove(oldRoute);
    _arrived(newRoute);
  }

  @override
  void didPop(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      _revealed(route, previousRoute);

  @override
  void didRemove(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      _revealed(route, previousRoute);

  void _arrived(Route<dynamic>? route) {
    if (!_trail._reporter.reporting || !_isScreen(route)) return;
    final page = route! as PageRoute<dynamic>;
    _pages.add(page);
    _trail._announce(page);
  }

  void _revealed(Route<dynamic> left, Route<dynamic>? revealed) {
    _pages.remove(left);
    if (_isScreen(left) && _isScreen(revealed)) {
      _trail._announce(revealed! as PageRoute<dynamic>);
    }
  }
}
