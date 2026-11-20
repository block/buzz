import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../../shared/theme/theme.dart';
import '../../../shared/widgets/app_list.dart';
import '../../../shared/widgets/buzz_sheet_header.dart';
import '../../../shared/widgets/modal_presentation.dart';
import '../../../shared/widgets/ios_glass_navigation_button.dart';

final mobileSectionOrderProvider = FutureProvider.family<List<String>, String>(
  (ref, key) async =>
      (await SharedPreferences.getInstance()).getStringList(key) ?? [],
);

/// Captures the visible inventory, including built-ins and mixed workspaces.
class SectionOrderScope extends InheritedWidget {
  const SectionOrderScope({
    super.key,
    required this.labels,
    required this.storageKey,
    required super.child,
  });
  final Map<String, String> labels;
  final String storageKey;

  static SectionOrderScope? of(BuildContext context) =>
      context.getInheritedWidgetOfExactType<SectionOrderScope>();

  @override
  bool updateShouldNotify(SectionOrderScope oldWidget) =>
      labels != oldWidget.labels || storageKey != oldWidget.storageKey;

  Future<void> show(BuildContext context) => showBuzzModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showCloseButton: false,
    enableDrag: false,
    builder: (_) => _ReorderSheet(scope: this),
  );
}

class _ReorderSheet extends HookConsumerWidget {
  const _ReorderSheet({required this.scope});
  final SectionOrderScope scope;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final draft = useState(scope.labels.keys.toList());
    final saving = useState(false);
    final failed = useState(false);
    final scroll = useScrollController();
    final rowContexts = useMemoized(() => <int, BuildContext>{});
    final dragIndex = useRef<int?>(null);
    final lastSlot = useRef(0);
    final pointer = useRef<int?>(null);
    final pointerY = useRef(0.0);
    final origin = useRef(0.0);
    final heights = useMemoized(() => <int, double>{});

    void updateSlot() {
      final start = dragIndex.value;
      if (start == null || !scroll.hasClients) return;
      final travel = pointerY.value + scroll.offset - origin.value;
      final direction = travel < 0 ? -1 : 1;
      var remaining = travel.abs();
      var slot = start;
      for (
        var i = start + direction;
        i >= 0 && i < draft.value.length;
        i += direction
      ) {
        final row = rowContexts[i];
        final height = row != null && row.mounted ? row.size?.height : null;
        if (height != null) heights[i] = height;
        final extent = heights[i];
        if (extent == null || remaining < extent / 2) break;
        slot = i;
        remaining -= extent;
      }
      if (slot != lastSlot.value) {
        lastSlot.value = slot;
        unawaited(HapticFeedback.selectionClick());
      }
    }

    useEffect(() {
      scroll.addListener(updateSlot);
      return () => scroll.removeListener(updateSlot);
    }, [scroll]);

    Future<void> save() async {
      if (saving.value) return;
      saving.value = true;
      try {
        final prefs = await SharedPreferences.getInstance();
        if (!await prefs.setStringList(scope.storageKey, draft.value)) {
          throw StateError('Unable to save section order');
        }
        if (!context.mounted) return;
        ref.invalidate(mobileSectionOrderProvider(scope.storageKey));
        Navigator.pop(context);
      } catch (_) {
        if (context.mounted) failed.value = true;
      } finally {
        if (context.mounted) saving.value = false;
      }
    }

    return PopScope(
      canPop: !saving.value,
      child: ConstrainedBox(
        constraints: BoxConstraints(
          maxHeight: MediaQuery.sizeOf(context).height * .85,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            BuzzSheetHeader(
              title: 'Reorder sections',
              leading: defaultTargetPlatform == TargetPlatform.iOS
                  ? IosGlassNavigationButton(
                      icon: IosGlassNavigationIcon.close,
                      semanticLabel: 'Discard order',
                      onPressed: saving.value
                          ? null
                          : () => Navigator.pop(context),
                    )
                  : IconButton(
                      tooltip: 'Discard order',
                      onPressed: saving.value
                          ? null
                          : () => Navigator.pop(context),
                      icon: const Icon(Icons.close),
                    ),
              trailing: defaultTargetPlatform == TargetPlatform.iOS
                  ? IosGlassNavigationButton(
                      icon: IosGlassNavigationIcon.check,
                      semanticLabel: 'Save order',
                      onPressed: saving.value ? null : save,
                    )
                  : IconButton(
                      tooltip: 'Save order',
                      onPressed: saving.value ? null : save,
                      icon: const Icon(Icons.check),
                    ),
            ),
            Flexible(
              child: Padding(
                padding: const EdgeInsets.symmetric(
                  horizontal: Grid.gutter,
                  vertical: Grid.xxs,
                ),
                child: Material(
                  color: context.colors.surfaceContainerHighest,
                  borderRadius: BorderRadius.circular(Radii.container),
                  clipBehavior: Clip.antiAlias,
                  child: Listener(
                    onPointerMove: (event) {
                      if (event.pointer != pointer.value) return;
                      pointerY.value = event.position.dy;
                      updateSlot();
                    },
                    child: ReorderableListView.builder(
                      scrollController: scroll,
                      onReorderStart: (index) {
                        dragIndex.value = index;
                        lastSlot.value = index;
                        unawaited(HapticFeedback.mediumImpact());
                      },
                      onReorderEnd: (_) => dragIndex.value = null,
                      shrinkWrap: true,
                      buildDefaultDragHandles: false,
                      itemCount: draft.value.length,
                      onReorderItem: (oldIndex, newIndex) {
                        final next = draft.value.toList();
                        next.insert(newIndex, next.removeAt(oldIndex));
                        draft.value = next;
                      },
                      itemBuilder: (_, index) {
                        final key = draft.value[index];
                        return Builder(
                          key: ValueKey(key),
                          builder: (rowContext) {
                            rowContexts[index] = rowContext;
                            return Column(
                              children: [
                                if (index > 0)
                                  Divider(
                                    height: 1,
                                    indent: Grid.gutter,
                                    endIndent: Grid.gutter,
                                    color: context.colors.onSurface.withValues(
                                      alpha: .07,
                                    ),
                                  ),
                                AppListRow(
                                  verticalPadding: Grid.xxs,
                                  title: scope.labels[key]!,
                                  trailing: Listener(
                                    onPointerDown: (event) {
                                      if (dragIndex.value != null) return;
                                      pointer.value = event.pointer;
                                      pointerY.value = event.position.dy;
                                      origin.value =
                                          event.position.dy + scroll.offset;
                                      heights.clear();
                                    },
                                    child: ReorderableDragStartListener(
                                      index: index,
                                      enabled: !saving.value,
                                      child: const SizedBox.square(
                                        dimension: 44,
                                        child: Icon(
                                          Icons.drag_handle,
                                          size: 20,
                                        ),
                                      ),
                                    ),
                                  ),
                                ),
                              ],
                            );
                          },
                        );
                      },
                    ),
                  ),
                ),
              ),
            ),
            if (failed.value)
              const Text('Could not save section order. Try again.'),
          ],
        ),
      ),
    );
  }
}
