import 'dart:math' as math;

import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/material.dart';

/// Keeps route surfaces full width; each pane reserves its controls separately.
class DuoWorkspaceScaffold extends StatelessWidget {
  const DuoWorkspaceScaffold({
    required this.body,
    required this.navigationBuilder,
    super.key,
  });

  final Widget body;
  final FoldNavigationBuilder navigationBuilder;

  @override
  Widget build(BuildContext context) {
    final strip = FoldGeometry.of(context).strip;
    final placement = switch (strip?.side) {
      FoldStripSide.left => FoldNavigationPlacement.left,
      FoldStripSide.right => FoldNavigationPlacement.right,
      null => FoldNavigationPlacement.bottom,
    };
    return Stack(
      fit: StackFit.expand,
      children: [
        body,
        if (strip != null)
          Positioned.fromRect(
            rect: strip.rect,
            child: MediaQuery(
              data: MediaQuery.of(context).copyWith(
                padding: EdgeInsets.only(
                  top: strip.topClearance,
                  bottom: math.max(
                    strip.bottomClearance,
                    MediaQuery.viewPaddingOf(context).bottom,
                  ),
                ),
              ),
              child: navigationBuilder(context, placement),
            ),
          ),
      ],
    );
  }
}
