import 'package:flutter/widgets.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Holds [child]'s keyboard insets steady while its route is covered.
///
/// A keyboard raised by a route above (a sheet with a search field, or a
/// native picker presented over the app) still changes the app's
/// [MediaQuery] on every animation frame. Without this, a covered page that
/// reads `MediaQuery.of`, `paddingOf` or `viewInsetsOf` (directly or through
/// a [Scaffold]) rebuilds and relays out its whole content for keyboard
/// motion it cannot show. While the route is not current, [child] keeps the
/// view insets and padding it last had as the current route, except that it
/// still follows the keyboard down: when a covering route takes focus and the
/// page's own keyboard closes, the page lowers behind the scrim instead of
/// jumping when it is uncovered. A size change (rotation) drops the held
/// values, so safe-area padding stays right for the new orientation. Every
/// other [MediaQueryData] field passes through.
///
/// When the route becomes current again with the keyboard still above the
/// held value (the covering route's keyboard has not finished closing), the
/// page keeps the held values instead of jumping up to the live keyboard and
/// riding it back down. It releases to the live values once the keyboard
/// comes down to the held value, or as soon as focus is inside this route
/// (the page raised its own keyboard and has to follow it).
class CoveredRouteKeyboardInsets extends HookWidget {
  const CoveredRouteKeyboardInsets({
    super.key,
    required this.child,
    this.isCovered,
  });

  final Widget child;

  /// Overrides route currency for content covered within its own route, such
  /// as a compact list beneath a nested detail navigator.
  final bool? isCovered;

  @override
  Widget build(BuildContext context) {
    final data = MediaQuery.of(context);
    final current = isCovered == null
        ? ModalRoute.isCurrentOf(context) ?? true
        : !isCovered!;
    // Focus moving into the route must release the held values even when the
    // keyboard height is not changing at that moment.
    useListenable(FocusManager.instance);
    final focusInside = _hasFocusInside(FocusScope.of(context));
    // The insets and padding from the last build as the current route, or
    // the smallest keyboard seen since it was covered at the current size.
    final held = useRef<(EdgeInsets, EdgeInsets, Size)?>(null);
    // Whether the last build was as a covered route.
    final covered = useRef(false);
    // Whether the route is current again but still shows the held values.
    final settling = useRef(false);
    final heldValue = held.value;
    final sizeChanged = heldValue != null && data.size != heldValue.$3;
    if (!current) {
      covered.value = true;
      settling.value = false;
      if (heldValue == null ||
          sizeChanged ||
          data.viewInsets.bottom <= heldValue.$1.bottom) {
        held.value = (data.viewInsets, data.padding, data.size);
      }
    } else {
      if (covered.value &&
          heldValue != null &&
          !sizeChanged &&
          !focusInside &&
          data.viewInsets.bottom > heldValue.$1.bottom) {
        settling.value = true;
      }
      covered.value = false;
      if (settling.value &&
          (sizeChanged ||
              focusInside ||
              data.viewInsets.bottom <= heldValue!.$1.bottom)) {
        settling.value = false;
      }
      if (!settling.value) {
        held.value = (data.viewInsets, data.padding, data.size);
      }
    }
    final (viewInsets, padding, _) = held.value!;
    // Always the same widget shape, so covering never remounts the page.
    return MediaQuery(
      data: current && !settling.value
          ? data
          : data.copyWith(viewInsets: viewInsets, padding: padding),
      child: child,
    );
  }

  /// Whether primary focus is on a descendant of [scope], not [scope] itself
  /// (which holds focus when nothing inside the route has it).
  static bool _hasFocusInside(FocusScopeNode scope) {
    final primary = FocusManager.instance.primaryFocus;
    return primary != null &&
        primary != scope &&
        primary.ancestors.contains(scope);
  }
}
