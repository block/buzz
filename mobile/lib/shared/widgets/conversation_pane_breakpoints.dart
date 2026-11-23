import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/widgets.dart';

import 'ios_navigation_metrics.dart';

/// Duo uses the phone navigation model in portrait, even on its large display.
/// Callers inside a pane supply the full window size.
bool usesConversationPanes(
  BuildContext context, {
  FoldStrip? strip,
  Size? windowSize,
}) {
  final size = windowSize ?? MediaQuery.sizeOf(context);
  final rail = strip ?? FoldGeometry.of(context).strip;
  // The portrait Duo status cluster becomes a tall top inset, not a side
  // strip. Keep that phone layout compact without excluding ordinary iPads.
  final phoneStatusCluster = MediaQuery.viewPaddingOf(context).top > 50;
  if (defaultTargetPlatform == TargetPlatform.iOS &&
      size.height >= size.width &&
      (rail != null || phoneStatusCluster || size.shortestSide < 600)) {
    return false;
  }
  final bookDivision = bookDivisionOf(context);
  // A landscape phone is wide enough for the breakpoint, but not for two
  // usable panes. Fold geometry keeps Duo eligible in its supported poses.
  if (size.shortestSide < 600 && rail == null && bookDivision == null) {
    return false;
  }
  return bookDivision != null || size.width - (rail?.width ?? 0) >= 640;
}

/// The active vertical fold, in window coordinates, when held like a book.
Rect? bookDivisionOf(BuildContext context) {
  final geometry = FoldGeometry.of(context);
  final division = geometry.division;
  if (division != null &&
      division.axis == Axis.vertical &&
      division.isHalfOpened) {
    return division.bounds;
  }
  final native = IosNavigationMetrics.of(context);
  return native.windowSize == geometry.size ? native.bookDivision : null;
}
