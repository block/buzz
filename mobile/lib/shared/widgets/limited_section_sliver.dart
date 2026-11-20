import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import '../theme/buzz_icons.dart';
import 'collapsible_section_body.dart';

/// A compact inventory with the same reveal motion as its enclosing section.
class LimitedSectionSliver extends HookWidget {
  const LimitedSectionSliver({
    required this.itemCount,
    required this.itemBuilder,
    this.horizontalInset = Grid.gutter,
    this.findChildIndexCallback,
    this.revealAll = false,
    this.overflowActivity,
    super.key,
  });

  static const visibleLimit = 7;
  final bool revealAll;
  final String? overflowActivity;
  final int itemCount;
  final IndexedWidgetBuilder itemBuilder;
  final double horizontalInset;
  final ChildIndexGetter? findChildIndexCallback;

  @override
  Widget build(BuildContext context) {
    final showAll = useState(false);
    final startKey = useMemoized(GlobalKey.new);
    final initialCount = revealAll
        ? itemCount
        : math.min(itemCount, visibleLimit);
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    return SliverMainAxisGroup(
      slivers: [
        SliverToBoxAdapter(child: SizedBox(key: startKey)),
        TweenAnimationBuilder<double>(
          tween: Tween(end: showAll.value ? 1.0 : 0.0),
          duration: reducedMotion
              ? Duration.zero
              : showAll.value
              ? sectionExpandDuration
              : sectionCollapseDuration,
          curve: showAll.value ? sectionExpandCurve : sectionCollapseCurve,
          builder: (context, progress, _) {
            final extent = initialCount + progress * (itemCount - initialCount);
            final count = extent.ceil().clamp(0, itemCount);
            return SliverList.builder(
              itemCount: count,
              findChildIndexCallback: findChildIndexCallback == null
                  ? null
                  : (key) {
                      if (key is! ValueKey<Key>) return null;
                      final index = findChildIndexCallback!(key.value);
                      return index != null && index >= 0 && index < count
                          ? index
                          : null;
                    },
              itemBuilder: (context, index) {
                final row = itemBuilder(context, index);
                return ClipRect(
                  key: row.key == null ? null : ValueKey<Key>(row.key!),
                  child: Align(
                    alignment: Alignment.topCenter,
                    heightFactor: (extent - index).clamp(0.0, 1.0),
                    child: row,
                  ),
                );
              },
            );
          },
        ),
        if (!revealAll && itemCount > visibleLimit) ...[
          SliverToBoxAdapter(
            child: Semantics(
              expanded: showAll.value,
              button: true,
              child: InkWell(
                borderRadius: BorderRadius.circular(Radii.md),
                onTap: () {
                  if (showAll.value) {
                    final target = startKey.currentContext?.findRenderObject();
                    final position = Scrollable.of(context).position;
                    if (target != null) {
                      final offset = RenderAbstractViewport.of(
                        target,
                      ).getOffsetToReveal(target, 0).offset;
                      if (position.pixels > offset) {
                        position.jumpTo(
                          offset.clamp(
                            position.minScrollExtent,
                            position.maxScrollExtent,
                          ),
                        );
                      }
                    }
                  }
                  showAll.value = !showAll.value;
                },
                child: Padding(
                  padding: EdgeInsets.fromLTRB(
                    horizontalInset,
                    Grid.inventoryRowInset,
                    horizontalInset,
                    Grid.inventoryRowInset,
                  ),
                  child: Row(
                    children: [
                      SizedBox(
                        width: 24,
                        height: 22,
                        child: Align(
                          alignment: Alignment.centerLeft,
                          child: Icon(
                            showAll.value
                                ? BuzzIcons.chevronUp
                                : BuzzIcons.chevronDown,
                            size: 20,
                            color: navigationMetadataForeground(context),
                          ),
                        ),
                      ),
                      const SizedBox(width: Grid.inventoryLabelGap),
                      Expanded(
                        child: Text(
                          showAll.value ? 'Show fewer' : 'Show more',
                          style: navigationChatTitleTextStyle.copyWith(
                            color: navigationMetadataForeground(context),
                          ),
                        ),
                      ),
                      if (!showAll.value && overflowActivity != null)
                        Text(
                          overflowActivity!,
                          style: Theme.of(context).textTheme.labelMedium,
                        ),
                    ],
                  ),
                ),
              ),
            ),
          ),
        ],
      ],
    );
  }
}
