import 'dart:async';
import 'dart:math' as math;
import 'dart:ui' show ImageFilter;

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import '../theme/buzz_icons.dart';
import 'navigation_button_surface.dart';

/// Expands an Android shortcut into a rounded, translucent action menu.
class AndroidExpandingMenuButton extends HookWidget {
  const AndroidExpandingMenuButton({
    required this.actions,
    this.label = 'New',
    this.trigger,
    this.width = 112,
    this.height = 56,
    this.dividerBefore = const {},
    super.key,
  });

  final String label;
  final Widget? trigger;
  final double width, height;
  final Set<int> dividerBefore;

  final List<({String label, String symbol, VoidCallback? onPressed})> actions;

  @override
  Widget build(BuildContext context) {
    final open = useState(false);
    return IgnorePointer(
      ignoring: open.value,
      child: Opacity(
        opacity: open.value ? 0 : 1,
        child: NavigationButtonSurface(
          child: Builder(
            builder: (buttonContext) => Tooltip(
              message: label,
              child: TextButton(
                style: TextButton.styleFrom(
                  padding: EdgeInsets.zero,
                  minimumSize: Size.zero,
                  tapTargetSize: MaterialTapTargetSize.shrinkWrap,
                  foregroundColor: context.colors.onSurface,
                ),
                onPressed: () async {
                  final navigator = Navigator.of(context);
                  final box = buttonContext.findRenderObject() as RenderBox;
                  final overlay =
                      navigator.overlay!.context.findRenderObject()
                          as RenderBox;
                  final rect = MatrixUtils.transformRect(
                    box.getTransformTo(overlay),
                    Offset.zero & box.size,
                  );
                  final captured = actions;
                  final route = _CreateMenuRoute(
                    anchor: rect,
                    label: label,
                    trigger: trigger ?? const _NewLabel(),
                    dividerBefore: dividerBefore,
                    actions: captured,
                    reducedMotion: MediaQuery.disableAnimationsOf(context),
                    dismissLabel: MaterialLocalizations.of(
                      context,
                    ).modalBarrierDismissLabel,
                  );
                  unawaited(HapticFeedback.lightImpact());
                  open.value = true;
                  try {
                    final index = await navigator.push<int>(route);
                    await route.completed;
                    if (!context.mounted) return;
                    open.value = false;
                    if (index != null) captured[index].onPressed?.call();
                  } finally {
                    if (context.mounted) open.value = false;
                  }
                },
                child: SizedBox(
                  width: width,
                  height: height,
                  child: Center(child: trigger ?? const _NewLabel()),
                ),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

class _NewLabel extends StatelessWidget {
  const _NewLabel();
  @override
  Widget build(BuildContext context) => Row(
    mainAxisAlignment: MainAxisAlignment.center,
    children: const [
      Icon(BuzzIcons.plus, size: 24),
      SizedBox(width: Grid.xxs),
      Text('New'),
    ],
  );
}

class _CreateMenuRoute extends PopupRoute<int> {
  _CreateMenuRoute({
    required this.anchor,
    required this.label,
    required this.trigger,
    required this.dividerBefore,
    required this.actions,
    required this.reducedMotion,
    required this.dismissLabel,
  });
  final Rect anchor;
  final String label;
  final Widget trigger;
  final Set<int> dividerBefore;
  final List<({String label, String symbol, VoidCallback? onPressed})> actions;
  final bool reducedMotion;
  final String dismissLabel;
  @override
  Color? get barrierColor => null;
  @override
  bool get barrierDismissible => true;
  @override
  String get barrierLabel => dismissLabel;
  @override
  Duration get transitionDuration =>
      Duration(milliseconds: reducedMotion ? 0 : 240);
  @override
  Duration get reverseTransitionDuration =>
      Duration(milliseconds: reducedMotion ? 0 : 180);

  @override
  Widget buildPage(
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
  ) {
    final media = MediaQuery.of(context);
    final width = math.min(
      240.0,
      media.size.width - media.padding.horizontal - Grid.lg,
    );
    final rowHeight = math.max(48.0, media.textScaler.scale(20) + Grid.sm);
    final height = math.min(
      actions.length * rowHeight + Grid.xs + dividerBefore.length,
      media.size.height - media.padding.vertical - Grid.lg,
    );
    final expandDown = anchor.center.dy < media.size.height / 2;
    final left =
        (expandDown ? anchor.right - width : anchor.center.dx - width / 2)
            .clamp(
              media.padding.left + Grid.xs,
              media.size.width - media.padding.right - Grid.xs - width,
            )
            .toDouble();
    final bottom = anchor.bottom
        .clamp(
          media.padding.top + Grid.xs + height,
          media.size.height - media.padding.bottom,
        )
        .toDouble();
    final top = expandDown
        ? anchor.top
              .clamp(
                media.padding.top,
                media.size.height - media.padding.bottom - height,
              )
              .toDouble()
        : bottom - height;
    final destination = Rect.fromLTWH(left, top, width, height);
    return AnimatedBuilder(
      animation: animation,
      builder: (context, _) {
        final t = Curves.easeOutCubic.transform(animation.value);
        final rect = Rect.lerp(anchor, destination, t)!;
        final contentOpacity = ((t - .25) / .75).clamp(0.0, 1.0);
        return Stack(
          children: [
            Positioned.fromRect(
              rect: rect,
              child: DecoratedBox(
                decoration: BoxDecoration(
                  borderRadius: BorderRadius.circular(Radii.navigationCapsule),
                  boxShadow: [
                    BoxShadow(
                      color: context.colors.shadow.withValues(alpha: .16 * t),
                      blurRadius: 24 * t,
                      offset: Offset(0, 6 * t),
                    ),
                  ],
                ),
                child: ClipRRect(
                  borderRadius: BorderRadius.circular(Radii.navigationCapsule),
                  child: BackdropFilter(
                    filter: ImageFilter.blur(sigmaX: 16 * t, sigmaY: 16 * t),
                    child: Material(
                      color: context.controlSurface.withValues(
                        alpha: 1 - .12 * t,
                      ),
                      child: Stack(
                        children: [
                          Positioned.fill(
                            child: IgnorePointer(
                              child: Opacity(
                                opacity: (1 - t * 4).clamp(0.0, 1.0),
                                child: Center(child: trigger),
                              ),
                            ),
                          ),
                          Positioned.fill(
                            child: OverflowBox(
                              alignment: expandDown
                                  ? Alignment.topCenter
                                  : Alignment.bottomCenter,
                              minWidth: width,
                              maxWidth: width,
                              minHeight: height,
                              maxHeight: height,
                              child: Opacity(
                                opacity: contentOpacity,
                                child: Semantics(
                                  scopesRoute: true,
                                  namesRoute: true,
                                  label: label,
                                  explicitChildNodes: true,
                                  child: SingleChildScrollView(
                                    padding: const EdgeInsets.symmetric(
                                      vertical: Grid.xxs,
                                    ),
                                    child: Column(
                                      mainAxisSize: MainAxisSize.min,
                                      children: [
                                        for (
                                          var i = 0;
                                          i < actions.length;
                                          i++
                                        ) ...[
                                          if (dividerBefore.contains(i))
                                            const Divider(height: 1),
                                          SizedBox(
                                            height: rowHeight,
                                            child: InkWell(
                                              onTap:
                                                  actions[i].onPressed == null
                                                  ? null
                                                  : () => Navigator.of(
                                                      context,
                                                    ).pop(i),
                                              child: Padding(
                                                padding:
                                                    const EdgeInsets.symmetric(
                                                      horizontal: Grid.gutter,
                                                    ),
                                                child: Row(
                                                  children: [
                                                    Icon(switch (actions[i]
                                                        .symbol) {
                                                      '' => null,
                                                      'checkmark' =>
                                                        BuzzIcons.check,
                                                      'star' =>
                                                        Icons.star_border,
                                                      'info.circle' =>
                                                        Icons.info_outline,
                                                      'desktopcomputer' =>
                                                        Icons
                                                            .desktop_windows_outlined,
                                                      'square.and.pencil' =>
                                                        BuzzIcons.edit,
                                                      'folder.badge.plus' =>
                                                        BuzzIcons.folderPlus,
                                                      'bubble.left' =>
                                                        BuzzIcons.messageSquare,
                                                      'number' =>
                                                        BuzzIcons.hash,
                                                      'arrow.left.arrow.right' =>
                                                        BuzzIcons
                                                            .arrowLeftRight,
                                                      'person.badge.plus' =>
                                                        BuzzIcons.userPlus,
                                                      'circle.lefthalf.filled' =>
                                                        BuzzIcons.sunMoon,
                                                      'person.2' =>
                                                        BuzzIcons.users,
                                                      'gearshape' =>
                                                        BuzzIcons.settings,
                                                      _ => BuzzIcons.plus,
                                                    }, size: 22),
                                                    const SizedBox(
                                                      width: Grid.sm,
                                                    ),
                                                    Expanded(
                                                      child: Text(
                                                        actions[i].label,
                                                        style: TextStyle(
                                                          color:
                                                              actions[i]
                                                                      .onPressed ==
                                                                  null
                                                              ? context
                                                                    .colors
                                                                    .onSurface
                                                                    .withValues(
                                                                      alpha: .4,
                                                                    )
                                                              : context
                                                                    .colors
                                                                    .onSurface,
                                                        ),
                                                      ),
                                                    ),
                                                  ],
                                                ),
                                              ),
                                            ),
                                          ),
                                        ],
                                      ],
                                    ),
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
                ),
              ),
            ),
          ],
        );
      },
    );
  }
}
