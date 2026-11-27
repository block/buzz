import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../../shared/theme/buzz_icons.dart';
import '../../../shared/theme/theme.dart';
import '../../../shared/widgets/collapsible_section_body.dart';
import '../../../shared/widgets/collapsible_sliver_body.dart';
import '../channel.dart';

class _FilteredRowKey extends ValueKey<String> {
  const _FilteredRowKey(super.value);
}

/// Lazily reveals chats that fall outside a section's active filter.
class FilteredChannelSectionBody extends HookWidget {
  const FilteredChannelSectionBody({
    super.key,
    required this.expanded,
    required this.channels,
    required this.isVisible,
    required this.buildRow,
    this.emptySliver,
    this.disclosureInset = Grid.gutter,
  });

  final bool expanded;
  final List<Channel> channels;
  final bool Function(Channel) isVisible;
  final Widget Function(Channel) buildRow;
  final Widget? emptySliver;
  final double disclosureInset;

  @override
  Widget build(BuildContext context) {
    final visibleChannels = <Channel>[];
    final hiddenIds = <String>{};
    for (final channel in channels) {
      if (isVisible(channel)) {
        visibleChannels.add(channel);
      } else {
        hiddenIds.add(channel.id);
      }
    }
    final hiddenCount = hiddenIds.length;
    final reveal = useState(false);
    final controller = useAnimationController(
      duration: sectionExpandDuration,
      reverseDuration: sectionCollapseDuration,
    );
    final motion = useMemoized(
      () => CurvedAnimation(
        parent: controller,
        curve: sectionExpandCurve,
        reverseCurve: sectionCollapseCurve,
      ),
      [controller],
    );
    useEffect(() => motion.dispose, [motion]);
    // A sliver keeps laying out children until it fills the viewport. A
    // long run of nearly zero-height rows would mount the entire inventory.
    // Retain some layout extent on large reveals and fade those rows in.
    final largeReveal = hiddenCount > 24;
    final boundedMotion = useMemoized(
      () => Tween<double>(begin: 0.5, end: 1).animate(motion),
      [motion],
    );
    final renderingHidden = useState(false);
    useEffect(() {
      void onStatus(AnimationStatus status) {
        if (status == AnimationStatus.dismissed) renderingHidden.value = false;
      }

      controller.addStatusListener(onStatus);
      return () => controller.removeStatusListener(onStatus);
    }, [controller]);
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    useEffect(() {
      controller.value = 0;
      renderingHidden.value = false;
      reveal.value = false;
      return null;
    }, [hiddenIds.join(',')]);
    final displayed = renderingHidden.value ? channels : visibleChannels;
    final indexById = {
      for (final (index, channel) in displayed.indexed) channel.id: index,
    };

    return CollapsibleSliverBody(
      expanded: expanded,
      sliver: channels.isEmpty && emptySliver != null
          ? emptySliver!
          : SliverList.builder(
              itemCount: displayed.length + (hiddenCount == 0 ? 0 : 1),
              findChildIndexCallback: (key) =>
                  key is ValueKey<String> ? indexById[key.value] : null,
              itemBuilder: (context, index) {
                if (index < displayed.length) {
                  final channel = displayed[index];
                  final row = buildRow(channel);
                  if (!hiddenIds.contains(channel.id)) return row;
                  return SizeTransition(
                    key: _FilteredRowKey(channel.id),
                    sizeFactor: largeReveal ? boundedMotion : motion,
                    child: largeReveal
                        ? FadeTransition(opacity: motion, child: row)
                        : row,
                  );
                }
                final color = Theme.of(context).brightness == Brightness.dark
                    ? const Color(0xFF949494)
                    : const Color(0xFF707070);
                return Semantics(
                  expanded: reveal.value,
                  button: true,
                  child: InkWell(
                    borderRadius: BorderRadius.circular(Radii.md),
                    onTap: () {
                      reveal.value = !reveal.value;
                      if (reveal.value) renderingHidden.value = true;
                      if (reduceMotion) {
                        controller.value = reveal.value ? 1 : 0;
                        renderingHidden.value = reveal.value;
                      } else if (reveal.value) {
                        unawaited(controller.forward());
                      } else {
                        unawaited(controller.reverse());
                      }
                    },
                    child: Padding(
                      padding: EdgeInsets.symmetric(
                        horizontal: disclosureInset,
                        vertical: Grid.inventoryRowInset,
                      ),
                      child: Row(
                        children: [
                          SizedBox(
                            width: 24,
                            height: 22,
                            child: Align(
                              alignment: Alignment.centerLeft,
                              child: Icon(
                                reveal.value
                                    ? BuzzIcons.chevronUp
                                    : BuzzIcons.chevronDown,
                                size: 20,
                                color: color,
                              ),
                            ),
                          ),
                          const SizedBox(width: Grid.inventoryLabelGap),
                          Expanded(
                            child: Text(
                              reveal.value ? 'Show fewer' : 'Show more',
                              style: navigationChatTitleTextStyle.copyWith(
                                color: color,
                              ),
                            ),
                          ),
                        ],
                      ),
                    ),
                  ),
                );
              },
            ),
    );
  }
}
