import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import 'buzz_sheet_header.dart';
import 'concentric_sheet_surface.dart';

/// Shared solid sheet surface with a centered navigation row.
class BuzzTitledSheetLayout extends HookWidget {
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
    final isScrolledUnder = useState(false);
    final pendingScrolledUnder = useRef<bool?>(null);
    final scrollUpdateScheduled = useRef(false);
    final color = surfaceColor ?? context.colors.surface;
    final paintsSurface = !ConcentricSheetSurface.providesSurfaceOf(context);

    void updateScrollUnder(bool next) {
      if (scrollUpdateScheduled.value) {
        pendingScrolledUnder.value =
            (pendingScrolledUnder.value ?? false) || next;
        return;
      }
      pendingScrolledUnder.value = next;
      scrollUpdateScheduled.value = true;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        scrollUpdateScheduled.value = false;
        final pending = pendingScrolledUnder.value;
        pendingScrolledUnder.value = null;
        if (!context.mounted ||
            pending == null ||
            pending == isScrolledUnder.value) {
          return;
        }
        isScrolledUnder.value = pending;
      });
    }

    final sheet = SizedBox(
      width: double.infinity,
      child: ColoredBox(
        key: const ValueKey('buzz-sheet-surface'),
        color: paintsSurface ? color : Colors.transparent,
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            DecoratedBox(
              key: const ValueKey('buzz-sheet-scroll-divider'),
              decoration: BoxDecoration(
                border: Border(
                  bottom: BorderSide(
                    color: isScrolledUnder.value
                        ? navigationDivider(context, 0.05)
                        : Colors.transparent,
                  ),
                ),
              ),
              child: BuzzSheetHeader(
                title: title,
                titleKey: titleKey,
                leading: leading,
                trailing: trailing,
                showDragHandle: showDragHandle,
              ),
            ),
            Flexible(
              child: NotificationListener<ScrollNotification>(
                onNotification: (notification) {
                  if (notification.depth != 0 ||
                      notification.metrics.axis != Axis.vertical ||
                      (notification is! ScrollUpdateNotification &&
                          notification is! OverscrollNotification)) {
                    return false;
                  }
                  final next = notification.metrics.extentBefore > 0.5;
                  if (next != isScrolledUnder.value) updateScrollUnder(next);
                  return false;
                },
                child: child,
              ),
            ),
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
