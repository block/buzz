import 'package:flutter/material.dart';

import '../theme/theme.dart';

/// Keeps pointer, press and focus ink inside the sidebar selection gutter.
class SidebarRowInkBorder extends RoundedRectangleBorder {
  const SidebarRowInkBorder()
    : super(borderRadius: const BorderRadius.all(Radius.circular(24)));

  @override
  Path getOuterPath(Rect rect, {TextDirection? textDirection}) =>
      super.getOuterPath(
        const EdgeInsets.symmetric(horizontal: Grid.xxs).deflateRect(rect),
        textDirection: textDirection,
      );
}
