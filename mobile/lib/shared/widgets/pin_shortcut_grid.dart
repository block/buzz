import 'dart:async';
import 'dart:math' as math;

import 'package:duo_kit/duo_kit.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../theme/theme.dart';
import 'adaptive_conversation_pane.dart';
import 'duo_toolbar.dart';

const _compactSlotSize = 80.0;

/// Pin shortcuts wrap when the next portrait no longer fits in the card.
class PinShortcutGrid<T> extends HookWidget {
  const PinShortcutGrid({
    required this.items,
    required this.label,
    required this.onOpen,
    required this.onLongPress,
    required this.idOf,
    required this.artwork,
    required this.decorate,
    super.key,
  });
  final List<T> items;
  final String Function(T) label;
  final ValueChanged<T> onOpen;
  final ValueChanged<T> onLongPress;
  final String Function(T) idOf;
  final Widget Function(T) artwork;
  final Widget Function(BuildContext, Widget) decorate;

  static const _duration = Duration(milliseconds: 280);

  @override
  Widget build(BuildContext context) {
    final previous = usePrevious(items) ?? items;
    final held = useState<List<T>?>(null);
    final entering = useState(<String>{});
    final reflow = useRef<Timer?>(null);
    final reveal = useRef<Timer?>(null);
    useEffect(() {
      final ids = items.map((c) => idOf(c)).toSet();
      final added = ids.difference(previous.map((c) => idOf(c)).toSet());
      if (added.isNotEmpty) {
        entering.value = {...entering.value.where(ids.contains), ...added};
        reveal.value?.cancel();
        reveal.value = Timer(_duration, () => entering.value = {});
      }
      if (previous.any((c) => !ids.contains(idOf(c)))) {
        held.value ??= List.of(previous);
        reflow.value?.cancel();
        reflow.value = Timer(_duration, () => held.value = null);
      }
      if (held.value case final slots?) {
        final heldIds = slots.map((c) => idOf(c)).toSet();
        slots.addAll(items.where((c) => heldIds.add(idOf(c))));
      }
      return null;
    }, [items]);
    useEffect(
      () => () {
        reflow.value?.cancel();
        reveal.value?.cancel();
      },
      const [],
    );
    return items.isEmpty
        ? const SizedBox.shrink()
        : Padding(
            padding: const EdgeInsets.fromLTRB(
              Grid.xs,
              Grid.half,
              Grid.xs,
              Grid.xxs,
            ),
            child: LayoutBuilder(
              builder: (context, constraints) {
                const gap = Grid.twelve;
                final phone =
                    defaultTargetPlatform == TargetPlatform.iOS &&
                    MediaQuery.sizeOf(context).shortestSide < 600 &&
                    (DuoToolbarScope.maybeStripOf(context) ??
                            FoldGeometry.of(context).strip) ==
                        null;
                final slotSize = phone
                    ? math
                          .min(104.0, (constraints.maxWidth - 2 * gap) / 3)
                          .clamp(1.0, 104.0)
                          .toDouble()
                    : _compactSlotSize;
                final capacity = phone
                    ? 3
                    : math.max(
                        1,
                        ((constraints.maxWidth + gap) / (slotSize + gap))
                            .floor(),
                      );
                final duration = MediaQuery.disableAnimationsOf(context)
                    ? Duration.zero
                    : _duration;
                final current = {for (final item in items) idOf(item): item};
                final slots = duration == Duration.zero
                    ? items
                    : held.value ?? items;
                final count = slots.length;
                final columns = math.min(capacity, count);
                final split = count <= 2;
                final cellWidth = constraints.maxWidth / columns;
                final startInset = split
                    ? math.max(0.0, (cellWidth - slotSize) / 2)
                    : 0.0;
                final step = split
                    ? cellWidth
                    : columns == 1
                    ? 0.0
                    : (constraints.maxWidth - slotSize) / (columns - 1);
                const captionSize = 12.0;
                const captionLineHeight = 1.25;
                final captionColor =
                    Theme.of(context).brightness == Brightness.light &&
                        !MediaQuery.highContrastOf(context)
                    ? const Color(0xFF6D6D6D)
                    : context.colors.onSurfaceVariant;
                final captionHeight =
                    MediaQuery.textScalerOf(context).scale(captionSize) *
                    captionLineHeight;
                final rowHeight = slotSize + captionHeight.ceilToDouble();
                final rows = (count / columns).ceil();
                Widget slot(T item, int index, {bool leaving = false}) {
                  final name = label(item);
                  final hidden =
                      leaving ||
                      (duration != Duration.zero &&
                          entering.value.contains(idOf(item)));
                  void hold() => onLongPress(item);
                  return AnimatedPositionedDirectional(
                    key: ValueKey(idOf(item)),
                    duration: duration,
                    curve: Curves.easeOutQuad,
                    start: startInset + (index % columns) * step,
                    top: (index ~/ columns) * (rowHeight + gap),
                    child: IgnorePointer(
                      ignoring: hidden,
                      child: ExcludeSemantics(
                        excluding: hidden,
                        child: TweenAnimationBuilder<double>(
                          tween: Tween(begin: 0.0, end: hidden ? 0.0 : 1.0),
                          duration: duration,
                          curve: Curves.easeOutQuad,
                          builder: (_, scale, child) =>
                              Transform.scale(scale: scale, child: child),
                          child: decorate(
                            context,
                            Semantics(
                              button: true,
                              onLongPress: hold,
                              selected:
                                  ConversationPaneScope.maybeOf(
                                    context,
                                  )?.selectedId ==
                                  idOf(item),
                              onTap: () => onOpen(item),
                              label: 'Open pinned $name',
                              excludeSemantics: true,
                              child: Tooltip(
                                triggerMode: TooltipTriggerMode.manual,
                                message: name,
                                child: InkWell(
                                  onLongPress: hold,
                                  customBorder: const CircleBorder(),
                                  onTap: () => onOpen(item),
                                  child: SizedBox(
                                    width: slotSize,
                                    child: Column(
                                      mainAxisSize: MainAxisSize.min,
                                      children: [
                                        SizedBox.square(
                                          dimension: slotSize,
                                          child: FittedBox(
                                            child: SizedBox.square(
                                              dimension: _compactSlotSize,
                                              child: artwork(item),
                                            ),
                                          ),
                                        ),
                                        Transform.translate(
                                          // The portrait fills 80% of its square slot.
                                          offset: Offset(0, -slotSize * .05),
                                          child: Text(
                                            name,
                                            maxLines: 1,
                                            overflow: TextOverflow.ellipsis,
                                            textAlign: TextAlign.center,
                                            style: TextStyle(
                                              fontSize: captionSize,
                                              height: captionLineHeight,
                                              color: captionColor,
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
                        ),
                      ),
                    ),
                  );
                }

                return AnimatedContainer(
                  duration: duration,
                  curve: Curves.easeOutQuad,
                  height: rows * rowHeight + (rows - 1) * gap,
                  child: Stack(
                    children: [
                      for (var i = 0; i < count; i++)
                        slot(
                          current[idOf(slots[i])] ?? slots[i],
                          i,
                          leaving: !current.containsKey(idOf(slots[i])),
                        ),
                    ],
                  ),
                );
              },
            ),
          );
  }
}
