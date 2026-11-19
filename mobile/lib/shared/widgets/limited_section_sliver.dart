import 'dart:math' as math;

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import '../theme/buzz_icons.dart';
import 'collapsible_sliver_body.dart';

/// A compact inventory with the same reveal motion as its enclosing section.
class LimitedSectionSliver extends HookWidget {
  const LimitedSectionSliver({
    required this.itemCount,
    required this.itemBuilder,
    this.horizontalInset = Grid.gutter,
    this.findChildIndexCallback,
    this.revealAll = false,
    super.key,
  });

  static const visibleLimit = 7;
  final bool revealAll;
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
    int? childIndex(Key key, int start, int end) {
      final index = findChildIndexCallback?.call(key);
      return index == null || index < start || index >= end
          ? null
          : index - start;
    }

    return SliverMainAxisGroup(
      slivers: [
        SliverToBoxAdapter(child: SizedBox(key: startKey)),
        SliverList.builder(
          itemCount: initialCount,
          itemBuilder: itemBuilder,
          findChildIndexCallback: findChildIndexCallback == null
              ? null
              : (key) => childIndex(key, 0, initialCount),
        ),
        if (!revealAll && itemCount > visibleLimit) ...[
          CollapsibleSliverBody(
            expanded: showAll.value,
            sliver: SliverList.builder(
              itemCount: itemCount - initialCount,
              findChildIndexCallback: findChildIndexCallback == null
                  ? null
                  : (key) => childIndex(key, initialCount, itemCount),
              itemBuilder: (context, index) =>
                  itemBuilder(context, index + initialCount),
            ),
          ),
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
