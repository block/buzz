part of '../thread_detail_page.dart';

/// Reports the persistent head's extent for its placeholder in the lazy list.
class _ThreadHeadLayout extends HookWidget {
  final ValueChanged<double> onHeightChanged;
  final Widget child;

  const _ThreadHeadLayout({required this.onHeightChanged, required this.child});

  @override
  Widget build(BuildContext context) {
    final sizeKey = useMemoized(GlobalKey.new);
    final lastHeight = useRef<double?>(null);
    void reportHeight() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted) return;
        final box = sizeKey.currentContext?.findRenderObject();
        if (box is! RenderBox || !box.hasSize) return;
        final height = box.size.height;
        if (lastHeight.value == height) return;
        lastHeight.value = height;
        onHeightChanged(height);
      });
    }

    useEffect(() {
      reportHeight();
      return null;
    }, const []);
    return NotificationListener<SizeChangedLayoutNotification>(
      onNotification: (_) {
        reportHeight();
        return true;
      },
      child: SizeChangedLayoutNotifier(
        child: KeyedSubtree(key: sizeKey, child: child),
      ),
    );
  }
}

/// Gives the persistent head the same scroll input as its linked lazy list.
class _ThreadHeadScrollInput extends HookWidget {
  final bool enabled;
  final ScrollPosition? Function() scrollPosition;
  final ValueListenable<Iterable<ItemPosition>> positions;
  final double viewportTopEdge;
  final VoidCallback onUserScrollStart;
  final VoidCallback onUserScrollEnd;
  final Widget child;

  const _ThreadHeadScrollInput({
    required this.enabled,
    required this.scrollPosition,
    required this.positions,
    required this.viewportTopEdge,
    required this.onUserScrollStart,
    required this.onUserScrollEnd,
    required this.child,
  });

  @override
  Widget build(BuildContext context) {
    final headPositions = useValueListenable(positions);
    final headVisible =
        !enabled ||
        headPositions.any(
          (item) =>
              item.index == 0 &&
              item.itemLeadingEdge < 1 &&
              item.itemTrailingEdge > viewportTopEdge,
        );
    final drag = useRef<Drag?>(null);
    useEffect(
      () =>
          () => drag.value?.cancel(),
      [enabled],
    );
    final position = scrollPosition();
    useListenable(position);
    void userScroll(VoidCallback action) {
      onUserScrollStart();
      action();
      onUserScrollEnd();
    }

    void scrollPage(double direction) {
      final current = scrollPosition();
      if (current != null) {
        userScroll(() {
          current.moveTo(
            current.pixels + direction * current.viewportDimension * 0.8,
          );
        });
      }
    }

    return ExcludeSemantics(
      excluding: !headVisible,
      child: IgnorePointer(
        ignoring: !headVisible,
        child: Semantics(
          key: const ValueKey('thread-head-scroll-semantics'),
          sortKey: const OrdinalSortKey(-1),
          onScrollUp:
              enabled &&
                  position != null &&
                  position.pixels > position.minScrollExtent
              ? () => scrollPage(-1)
              : null,
          onScrollDown:
              enabled &&
                  position != null &&
                  position.pixels < position.maxScrollExtent
              ? () => scrollPage(1)
              : null,
          child: Listener(
            onPointerSignal: !enabled
                ? null
                : (event) {
                    if (event is PointerScrollEvent) {
                      GestureBinding.instance.pointerSignalResolver.register(
                        event,
                        (_) {
                          final current = scrollPosition();
                          if (current != null && event.scrollDelta.dy != 0) {
                            userScroll(() {
                              current.pointerScroll(event.scrollDelta.dy);
                            });
                          }
                        },
                      );
                    }
                  },
            child: GestureDetector(
              onVerticalDragStart: !enabled
                  ? null
                  : (details) {
                      drag.value = scrollPosition()?.drag(
                        details,
                        () => drag.value = null,
                      );
                    },
              onVerticalDragUpdate: !enabled
                  ? null
                  : (details) => drag.value?.update(details),
              onVerticalDragEnd: !enabled
                  ? null
                  : (details) => drag.value?.end(details),
              onVerticalDragCancel: !enabled
                  ? null
                  : () => drag.value?.cancel(),
              child: child,
            ),
          ),
        ),
      ),
    );
  }
}
