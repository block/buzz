import 'package:flutter/widgets.dart';

/// Holds [child]'s keyboard insets steady while its route is covered.
///
/// A keyboard raised by a route above (a sheet with a search field, or a
/// native picker presented over the app) still changes the app's
/// [MediaQuery] on every animation frame. Without this, a covered page that
/// reads `MediaQuery.of`, `paddingOf` or `viewInsetsOf` (directly or through
/// a [Scaffold]) rebuilds and relays out its whole content for keyboard
/// motion it cannot show. While the route is not current, [child] keeps the
/// view insets and padding it last had as the current route; every other
/// [MediaQueryData] field passes through. When the route becomes current
/// again the live values return.
class CoveredRouteKeyboardInsets extends StatefulWidget {
  const CoveredRouteKeyboardInsets({super.key, required this.child});

  final Widget child;

  @override
  State<CoveredRouteKeyboardInsets> createState() =>
      _CoveredRouteKeyboardInsetsState();
}

class _CoveredRouteKeyboardInsetsState
    extends State<CoveredRouteKeyboardInsets> {
  EdgeInsets? _viewInsets;
  EdgeInsets? _padding;

  @override
  Widget build(BuildContext context) {
    final data = MediaQuery.of(context);
    final current = ModalRoute.isCurrentOf(context) ?? true;
    if (current || _viewInsets == null || _padding == null) {
      _viewInsets = data.viewInsets;
      _padding = data.padding;
    }
    // Always the same widget shape, so covering never remounts the page.
    return MediaQuery(
      data: current
          ? data
          : data.copyWith(viewInsets: _viewInsets, padding: _padding),
      child: widget.child,
    );
  }
}
