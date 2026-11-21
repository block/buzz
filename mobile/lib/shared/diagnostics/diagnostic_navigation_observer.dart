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
/// [DiagnosticSheetRoute]s with no screen of their own: the screen they cover
/// is still the one shown, so opening or closing one leaves nothing new. A
/// screen equal to the last one left is skipped. A build
/// without a reporting destination does nothing, not even looking at routes.
class DiagnosticScreenTrail {
  DiagnosticScreenTrail(this._reporter, this._classifier);

  final DiagnosticReporter _reporter;
  final DiagnosticScreenClassifier Function() _classifier;
  final _observers = <DiagnosticNavigationObserver>{};
  DiagnosticScreen? _last;
  DiagnosticScreen? _cover;

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

  /// Shows [screen] in place of every navigator, such as the age restriction
  /// that replaces the app, or with null takes it away. While a cover shows,
  /// the navigators under it name nothing; taking it away names the screen
  /// shown again.
  void cover(DiagnosticScreen? screen) {
    if (screen == _cover) return;
    _cover = screen;
    if (!_reporter.reporting) return;
    if (screen != null) {
      _leave(screen);
    } else {
      for (final observer in _observers) {
        observer.refresh();
      }
    }
  }

  void _leave(DiagnosticScreen screen) {
    if (screen == _last) return;
    _last = screen;
    _reporter.leaveBreadcrumb(DiagnosticBreadcrumb.screen(screen));
  }

  /// Names [route] once the frame has built it, if it is then what the person
  /// sees. Several pops in one go, or a route replaced at once, leave only the
  /// screen they end on.
  void _announce(PageRoute<dynamic> route) {
    if (!_reporter.reporting) return;
    WidgetsBinding.instance.addPostFrameCallback((_) {
      if (_cover != null || !_shown(route)) return;
      _leave(screenOf(route));
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
      final page = observer._topPage;
      if (page != null && !page.isFirst) return page;
    }
    return null;
  }

  /// Whether [route] is the top screen of its navigator and of every
  /// navigator hosting it. A dialog or sheet over a screen leaves it shown.
  bool _shown(Route<dynamic> route) {
    final navigator = route.navigator;
    if (navigator == null || !route.isActive) return false;
    final top = _observerOf(navigator)?._topPage;
    if (top == null ? !route.isCurrent : top != route) return false;
    final host = ModalRoute.of(navigator.context);
    return host == null || _shown(host);
  }

  DiagnosticNavigationObserver? _observerOf(NavigatorState navigator) {
    for (final observer in _observers) {
      if (observer.navigator == navigator) return observer;
    }
    return null;
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

  /// The top screen, which shows even under a dialog or sheet.
  PageRoute<dynamic>? get _topPage {
    _pages.removeWhere((page) => !page.isActive);
    return _pages.lastOrNull;
  }

  /// Stops reporting this navigator.
  void dispose() => _trail._observers.remove(this);

  /// Names this navigator's current page again, once the frame has built it.
  /// Call it when a page's content changes in place with no navigation, such
  /// as the root page moving from the splash screen to onboarding or Home.
  void refresh() {
    if (_topPage case final page?) _trail._announce(page);
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
      _revealed(route);

  @override
  void didRemove(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      _revealed(route);

  void _arrived(Route<dynamic>? route) {
    if (!_trail._reporter.reporting || !_isScreen(route)) return;
    final page = route! as PageRoute<dynamic>;
    _pages.add(page);
    _trail._announce(page);
  }

  /// Names the top screen again after any route leaves, even a sheet: a
  /// screen can leave while covered, such as a channel opened over a sheet.
  void _revealed(Route<dynamic> left) {
    _pages.remove(left);
    if (_topPage case final page?) _trail._announce(page);
  }
}
