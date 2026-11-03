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
/// other [MediaQueryData] field passes through. When the route becomes
/// current again the live values return.
class CoveredRouteKeyboardInsets extends HookWidget {
  const CoveredRouteKeyboardInsets({super.key, required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context) {
    final data = MediaQuery.of(context);
    final current = ModalRoute.isCurrentOf(context) ?? true;
    // The insets and padding from the last build as the current route, or
    // the smallest keyboard seen since it was covered at the current size.
    final held = useRef<(EdgeInsets, EdgeInsets, Size)?>(null);
    final heldValue = held.value;
    if (current ||
        heldValue == null ||
        data.size != heldValue.$3 ||
        data.viewInsets.bottom <= heldValue.$1.bottom) {
      held.value = (data.viewInsets, data.padding, data.size);
    }
    final (viewInsets, padding, _) = held.value!;
    // Always the same widget shape, so covering never remounts the page.
    return MediaQuery(
      data: current
          ? data
          : data.copyWith(viewInsets: viewInsets, padding: padding),
      child: child,
    );
  }
}
