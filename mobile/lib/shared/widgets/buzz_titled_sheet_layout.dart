import 'package:flutter/material.dart';

import '../theme/theme.dart';
import 'buzz_sheet_header.dart';
import 'concentric_sheet_surface.dart';

/// Shared solid sheet surface with a centered navigation row.
class BuzzTitledSheetLayout extends StatelessWidget {
  const BuzzTitledSheetLayout({
    super.key,
    required this.title,
    required this.child,
    this.leading,
    this.trailing,
    this.titleKey,
    this.showDragHandle = false,
    this.surfaceColor,
  });

  final String title;
  final Widget child;
  final Widget? leading;
  final Widget? trailing;
  final Key? titleKey;
  final bool showDragHandle;
  final Color? surfaceColor;

  @override
  Widget build(BuildContext context) {
    final color = surfaceColor ?? context.colors.surface;
    final paintsSurface = !ConcentricSheetSurface.providesSurfaceOf(context);

    final sheet = SizedBox(
      width: double.infinity,
      child: ColoredBox(
        key: const ValueKey('buzz-sheet-surface'),
        color: paintsSurface ? color : Colors.transparent,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            BuzzSheetHeader(
              title: title,
              titleKey: titleKey,
              leading: leading,
              trailing: trailing,
              showDragHandle: showDragHandle,
            ),
            Flexible(child: child),
          ],
        ),
      ),
    );

    // The native iOS surface owns its exact iOS 26 container-concentric mask.
    // A second fixed Flutter radius would visibly square off those corners.
    if (!paintsSurface) return sheet;

    return ClipRRect(
      key: const ValueKey('buzz-sheet-surface-clip'),
      borderRadius: const BorderRadius.vertical(
        top: Radius.circular(Radii.dialog),
      ),
      clipBehavior: Clip.antiAlias,
      child: sheet,
    );
  }
}
