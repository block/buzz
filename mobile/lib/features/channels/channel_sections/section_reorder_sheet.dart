import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../../shared/relay/relay.dart';
import '../../../shared/theme/theme.dart';
import '../../../shared/widgets/app_list.dart';
import '../../../shared/widgets/buzz_sheet_header.dart';
import '../../../shared/widgets/ios_glass_navigation_button.dart';
import '../../../shared/widgets/modal_presentation.dart';

final mobileSectionOrderKeyProvider = Provider<String?>((ref) {
  final pubkey = ref.watch(myPubkeyProvider);
  if (pubkey == null) return null;
  final relay = ref.watch(
    relayConfigProvider.select((config) => config.baseUrl),
  );
  return 'section-order:$relay:$pubkey';
});

final mobileSectionOrderProvider =
    AsyncNotifierProvider.family<_MobileSectionOrder, List<String>, String>(
      _MobileSectionOrder.new,
    );

class _MobileSectionOrder extends AsyncNotifier<List<String>> {
  _MobileSectionOrder(this.key);

  final String key;
  bool _saving = false;

  @override
  Future<List<String>> build() async => List.unmodifiable(
    (await SharedPreferences.getInstance()).getStringList(key) ??
        const <String>[],
  );

  Future<bool> save(List<String> previous, List<String> next) async {
    if (_saving || !listEquals(state.asData?.value, previous)) return false;
    _saving = true;
    try {
      final prefs = await SharedPreferences.getInstance();
      if (!await prefs.setStringList(key, next)) {
        throw StateError('Unable to save section order');
      }
      if (ref.mounted) state = AsyncData(List.unmodifiable(next));
      return true;
    } finally {
      _saving = false;
    }
  }
}

List<String> resolveMobileSectionOrder(
  List<String> defaults,
  List<String> saved,
) => [
  ...{...saved.where(defaults.contains), ...defaults},
];

/// Captures the visible inventory, including built-ins and mixed workspaces.
class SectionOrderScope extends InheritedWidget {
  const SectionOrderScope({
    super.key,
    required this.labels,
    required this.defaultOrder,
    required this.storageKey,
    required super.child,
  });
  final Map<String, String> labels;
  final List<String> defaultOrder;
  final String storageKey;

  static SectionOrderScope? of(BuildContext context) =>
      context.getInheritedWidgetOfExactType<SectionOrderScope>();

  @override
  bool updateShouldNotify(SectionOrderScope oldWidget) =>
      !mapEquals(labels, oldWidget.labels) ||
      !listEquals(defaultOrder, oldWidget.defaultOrder) ||
      storageKey != oldWidget.storageKey;

  Future<void> show(BuildContext context) async {
    final container = ProviderScope.containerOf(context, listen: false);
    final route = ModalRoute.of(context);
    try {
      await container.read(mobileSectionOrderProvider(storageKey).future);
    } catch (_) {
      if (context.mounted && route?.isCurrent != false) {
        ScaffoldMessenger.maybeOf(context)?.showSnackBar(
          const SnackBar(
            content: Text('Could not load section order. Try again.'),
          ),
        );
      }
      return;
    }
    if (!context.mounted || route?.isCurrent == false) return;
    if (container.read(mobileSectionOrderKeyProvider) != storageKey) return;
    await showBuzzModalBottomSheet<void>(
      context: context,
      isScrollControlled: true,
      showCloseButton: false,
      enableDrag: false,
      builder: (_) => _ReorderSheet(scope: this),
    );
  }
}

class _ReorderSheet extends HookConsumerWidget {
  const _ReorderSheet({required this.scope});
  final SectionOrderScope scope;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final provider = mobileSectionOrderProvider(scope.storageKey);
    final saved = ref.watch(provider).value ?? const <String>[];
    final activeScope = ref.watch(mobileSectionOrderKeyProvider);
    final initialSaved = useMemoized(() => saved);
    final fullOrder = useMemoized(
      () => resolveMobileSectionOrder(scope.defaultOrder, initialSaved),
    );
    final initialVisible = useMemoized(
      () => fullOrder.where(scope.labels.containsKey).toList(),
    );
    final draft = useState(initialVisible);
    final saving = useState(false);
    final failed = useState(false);
    final orderChanged = !listEquals(saved, initialSaved);
    final scopeChanged = activeScope != scope.storageKey;
    final canEdit = !saving.value && !orderChanged && !scopeChanged;
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

    void move(int from, int to) {
      if (saving.value || !canEdit || from == to) return;
      final next = draft.value.toList();
      if (from < 0 || from >= next.length || to < 0 || to >= next.length) {
        return;
      }
      next.insert(to, next.removeAt(from));
      draft.value = next;
    }

    Future<void> save() async {
      if (saving.value || !canEdit || dragIndex.value != null) return;
      if (ref.read(mobileSectionOrderKeyProvider) != scope.storageKey) return;
      if (listEquals(draft.value, initialVisible)) {
        Navigator.pop(context);
        return;
      }
      saving.value = true;
      failed.value = false;
      final route = ModalRoute.of(context)!;
      try {
        final visible = draft.value.iterator;
        // Refill the visible slots in the new order and keep hidden slots
        // (such as an empty Starred section) in place.
        final next = [
          for (final key in fullOrder)
            if (scope.labels.containsKey(key) && visible.moveNext())
              visible.current
            else
              key,
        ];
        final persisted = await ref
            .read(provider.notifier)
            .save(initialSaved, next);
        if (!context.mounted) return;
        if (persisted) {
          if (route.isCurrent) {
            Navigator.pop(context);
          } else if (route.isActive) {
            Navigator.of(context).removeRoute(route);
          }
        } else {
          failed.value = true;
        }
      } catch (_) {
        if (context.mounted) failed.value = true;
      } finally {
        if (context.mounted) saving.value = false;
      }
    }

    void discard() {
      if (!saving.value) Navigator.pop(context);
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
                      onPressed: saving.value ? null : discard,
                    )
                  : IconButton(
                      tooltip: 'Discard order',
                      onPressed: saving.value ? null : discard,
                      icon: const Icon(Icons.close),
                    ),
              trailing: defaultTargetPlatform == TargetPlatform.iOS
                  ? IosGlassNavigationButton(
                      icon: IosGlassNavigationIcon.check,
                      semanticLabel: 'Save order',
                      onPressed: canEdit ? save : null,
                      isBusy: saving.value,
                    )
                  : IconButton(
                      tooltip: 'Save order',
                      onPressed: canEdit ? save : null,
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
                      onReorderItem: move,
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
                                    child: _SectionDragHandle(
                                      index: index,
                                      label: scope.labels[key]!,
                                      enabled: canEdit,
                                      onMove: (direction) {
                                        if (dragIndex.value != null) return;
                                        final from = draft.value.indexOf(key);
                                        move(from, from + direction);
                                      },
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
            if (!saving.value && (orderChanged || scopeChanged || failed.value))
              Semantics(
                liveRegion: true,
                child: Text(
                  scopeChanged
                      ? 'Account or community changed. Reopen section order to try again.'
                      : orderChanged
                      ? 'Section order changed. Reopen to try again.'
                      : 'Could not save section order. Try again.',
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _SectionDragHandle extends HookWidget {
  const _SectionDragHandle({
    required this.index,
    required this.label,
    required this.enabled,
    required this.onMove,
  });

  final int index;
  final String label;
  final bool enabled;
  final ValueChanged<int> onMove;

  @override
  Widget build(BuildContext context) {
    final highlighted = useState(false);
    return FocusableActionDetector(
      enabled: enabled,
      onShowFocusHighlight: (value) => highlighted.value = value,
      shortcuts: const {
        SingleActivator(LogicalKeyboardKey.arrowUp): DirectionalFocusIntent(
          TraversalDirection.up,
        ),
        SingleActivator(LogicalKeyboardKey.arrowDown): DirectionalFocusIntent(
          TraversalDirection.down,
        ),
      },
      actions: {
        DirectionalFocusIntent: CallbackAction<DirectionalFocusIntent>(
          onInvoke: (intent) {
            onMove(intent.direction == TraversalDirection.up ? -1 : 1);
            return null;
          },
        ),
      },
      child: Tooltip(
        message: 'Move $label with the up and down arrow keys',
        excludeFromSemantics: true,
        child: ReorderableDragStartListener(
          index: index,
          enabled: enabled,
          child: SizedBox.square(
            dimension: 44,
            child: DecoratedBox(
              decoration: BoxDecoration(
                borderRadius: BorderRadius.circular(Radii.sm),
                border: highlighted.value
                    ? Border.all(color: context.colors.primary)
                    : null,
              ),
              child: const Icon(Icons.drag_handle, size: 20),
            ),
          ),
        ),
      ),
    );
  }
}
