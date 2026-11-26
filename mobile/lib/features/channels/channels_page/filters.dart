export 'messages_display.dart';
import 'messages_display.dart';
import '../channel_sort/channel_sort_provider.dart';
import '../channel_sections/section_reorder_sheet.dart';
import '../../../shared/widgets/app_list.dart';
import '../channel_sections/channel_sections_provider.dart';
import 'package:flutter/material.dart';
import 'package:duo_kit/duo_kit.dart';
import '../../../shared/widgets/duo_toolbar.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../../../shared/theme/theme.dart';
import '../../../shared/theme/buzz_icons.dart';
import 'package:flutter/foundation.dart';
import '../../../shared/widgets/buzz_setting_menu.dart';
import '../../../shared/widgets/ios_glass_navigation_button.dart';
import '../../../shared/widgets/app_list_card.dart';
import '../../../shared/widgets/collapsible_section_body.dart';
import '../../../shared/widgets/buzz_sheet_header.dart';

import '../../../shared/widgets/modal_presentation.dart';
import '../../../shared/widgets/bottom_up_page_route.dart';
import '../channel_filter/channel_filter_provider.dart';
import '../channel_filter/channel_filter_store.dart';
import '../channel_sort/channel_sort_storage.dart';

Future<void> showMessagesDisplayOptions(BuildContext context) async {
  await showSectionViewSheet(context, title: 'Display Options');
}

class MessagesSectionOptions extends InheritedWidget {
  const MessagesSectionOptions({super.key, required super.child});

  static double messagesGutter(BuildContext context) {
    if (defaultTargetPlatform != TargetPlatform.iOS) return Grid.xs;
    final hasStrip =
        DuoToolbarScope.maybeStripOf(context) != null ||
        FoldGeometry.of(context).strip != null;
    return !hasStrip && isCompactDuoWindow(context) ? Grid.gutter : Grid.xs;
  }

  static double leadingInset(BuildContext context) =>
      active(context) ? messagesGutter(context) : Grid.gutter;

  static bool active(BuildContext context) =>
      context.dependOnInheritedWidgetOfExactType<MessagesSectionOptions>() !=
      null;
  @override
  bool updateShouldNotify(MessagesSectionOptions oldWidget) => false;
}

const showMessagesLayoutBounds = bool.fromEnvironment(
  'BUZZ_MESSAGES_LAYOUT_BOUNDS',
);

Widget markMessagesRow(BuildContext context, Widget child) {
  if (!showMessagesLayoutBounds || !MessagesSectionOptions.active(context)) {
    return child;
  }
  return DecoratedBox(
    position: DecorationPosition.foreground,
    decoration: BoxDecoration(
      color: const Color(0x0FFF0000),
      border: Border.all(color: const Color(0xFFD74A4A)),
    ),
    child: child,
  );
}

Future<String?> showSectionViewSheet(
  BuildContext context, {
  required String title,
  String? group,
  ChannelSortMode? sortMode,
  ValueChanged<ChannelSortMode>? onSort,
  bool custom = false,
}) async {
  final orderScope = context
      .getElementForInheritedWidgetOfExactType<SectionOrderScope>();
  final presentingRoute = defaultTargetPlatform == TargetPlatform.iOS
      ? ModalRoute.of(context)
      : null;
  final release = presentingRoute == null
      ? null
      : BottomUpPageRoute.retainControlsForSheet(presentingRoute);
  ModalRoute<dynamic>? sheetRoute;
  String? action;
  try {
    action = await showBuzzModalBottomSheet<String>(
      context: context,
      nativeIosBarrier: presentingRoute != null,
      showCloseButton: false,
      showDragHandle: false,
      isScrollControlled: true,
      isDismissible: true,
      enableDrag: false,
      builder: (sheetContext) {
        sheetRoute = ModalRoute.of(sheetContext);
        return _SectionViewSheet(
          centralized:
              group == null || context.widget is MessagesSectionOptions,
          canReorder: orderScope != null,
          title: title,
          group: group,
          initialSort: sortMode,
          onSort: onSort,
          custom: custom,
        );
      },
    );
    // Keep the same Home header mounted through both reverse transitions.
    final opensEditor = action == 'reorder' || action == 'editSections';
    if (release != null || opensEditor) await sheetRoute?.completed;
    if (opensEditor) {
      if (orderScope != null && orderScope.mounted) {
        await (orderScope.widget as SectionOrderScope).show(
          orderScope,
          editSections: action == 'editSections',
        );
      }
      return null;
    }
    return action;
  } finally {
    release?.call();
  }
}

class _SectionViewSheet extends HookConsumerWidget {
  final bool centralized;
  final bool canReorder;
  final String title;
  final String? group;
  final ChannelSortMode? initialSort;
  final ValueChanged<ChannelSortMode>? onSort;
  final bool custom;

  const _SectionViewSheet({
    required this.centralized,
    required this.canReorder,
    required this.title,
    required this.group,
    required this.initialSort,
    required this.onSort,
    required this.custom,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final group = this.group;
    final global = group == null;
    final display = global
        ? ref.watch(messagesDisplayProvider)
        : const MessagesDisplay();
    final scope = global ? ref.watch(mobileSectionOrderKeyProvider) : null;
    final initialScope = useRef(scope);
    final manager = global
        ? null
        : ref.watch(
            group.startsWith('solo:')
                ? soloFilterProvider
                : channelFilterProvider,
          );
    useListenable(manager);
    // Watched, not captured: a sheet opened before desktop's unified records
    // arrive turns read-only when they do.
    final sortEditable = ref.watch(
      channelSortProvider.select((state) => state.editable),
    );
    final layoutEditable = ref.watch(
      channelSectionsProvider.select((state) => state.editable),
    );
    final originalManager = useRef(manager);
    final sort = useState(global ? display.sort : initialSort);
    final mode = useState(
      global
          ? display.mode
          : manager?.store.modeFor(group) ?? ChannelFilterMode.all,
    );
    final count = useState(
      global
          ? display.count
          : manager?.store.countFor(group) ?? sectionCounts.first,
    );
    final days = useState(
      global
          ? display.days
          : (manager?.store.windowFor(group) ?? defaultActiveWindowSeconds) ~/
                86400,
    );
    final filterDirty = useState(false);
    final saving = useState(false);
    final failed = useState(false);
    // A confirmed initial load may arrive while the sheet is open. Preserve edits.
    useEffect(() {
      if (!global && !filterDirty.value && !saving.value) {
        count.value = manager?.store.countFor(group) ?? sectionCounts.first;
        mode.value = manager?.store.modeFor(group) ?? ChannelFilterMode.all;
        days.value =
            (manager?.store.windowFor(group) ?? defaultActiveWindowSeconds) ~/
            86400;
      }
      return null;
    }, [manager, manager?.ready, manager?.store]);
    if (scope != initialScope.value ||
        !identical(originalManager.value, manager)) {
      return SafeArea(
        child: Padding(
          padding: const EdgeInsets.all(Grid.xs),
          child: Column(
            mainAxisSize: MainAxisSize.min,
            children: [
              const Text(
                'Preferences changed. Reopen section options to continue.',
              ),
              TextButton(
                onPressed: () => Navigator.of(context).pop(),
                child: const Text('Close'),
              ),
            ],
          ),
        ),
      );
    }
    final writable =
        global ||
        (manager?.ready == true && manager?.busy == false && !saving.value);
    void close([String? action]) {
      if (!saving.value) Navigator.of(context).pop(action);
    }

    bool filterChanged() =>
        !global &&
        filterDirty.value &&
        (mode.value !=
                (manager?.store.modeFor(group) ?? ChannelFilterMode.all) ||
            (mode.value != ChannelFilterMode.active &&
                count.value != manager?.store.countFor(group)) ||
            (mode.value == ChannelFilterMode.active &&
                days.value! * 86400 != manager?.store.windowFor(group)));

    Future<void> apply([String? nextAction]) async {
      saving.value = true;
      failed.value = false;
      try {
        if (global) {
          await ref
              .read(messagesDisplayProvider.notifier)
              .update(
                MessagesDisplay(
                  sort: sort.value,
                  mode: mode.value,
                  count: count.value,
                  days: days.value,
                ),
              );
        } else if (filterChanged()) {
          if (manager == null || !manager.ready || manager.busy) {
            throw StateError('Preferences are read-only');
          }
          await manager.save(
            group,
            mode.value!,
            previewCount:
                mode.value != ChannelFilterMode.active &&
                    count.value != manager.store.countFor(group)
                ? count.value
                : null,
            windowSeconds: mode.value == ChannelFilterMode.active
                ? days.value! * 86400
                : null,
          );
        }
        if (!context.mounted) return;
        if (!global && sortEditable && sort.value != initialSort) {
          onSort?.call(sort.value!);
        }
        Navigator.of(context).pop(nextAction);
      } catch (_) {
        if (context.mounted) failed.value = true;
      } finally {
        if (context.mounted) saving.value = false;
      }
    }

    Widget choices<T>(
      String label,
      Map<T, String> values,
      T? selected,
      ValueChanged<T?>? change,
    ) => BuzzSettingMenu<T?>(
      controlKey: ValueKey(
        global ? 'messages-display-$label' : 'section-menu-$label',
      ),
      grouped: true,
      trailingIcon: Icons.unfold_more,
      enabled: change != null,
      label: label,
      value: selected,
      valueLabel: values[selected] ?? 'Section settings',
      onSelected: (value) => change?.call(value),
      options: [
        for (final entry in values.entries)
          BuzzSettingMenuOption(label: entry.value, value: entry.key),
      ],
    );
    final onApply = saving.value || (filterChanged() && manager?.busy == true)
        ? null
        : apply;
    final applyLabel = saving.value ? 'Saving…' : 'Apply changes';
    return PopScope(
      canPop: !saving.value,
      child: SafeArea(
        child: ConstrainedBox(
          constraints: BoxConstraints(
            maxHeight: MediaQuery.sizeOf(context).height * 0.85,
          ),
          child: Padding(
            padding: const EdgeInsets.only(bottom: Grid.twelve),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                BuzzSheetHeader(
                  title: title,
                  leading: defaultTargetPlatform == TargetPlatform.iOS
                      ? IosGlassNavigationButton(
                          icon: IosGlassNavigationIcon.close,
                          semanticLabel: 'Discard changes',
                          onPressed: saving.value ? null : () => close(),
                        )
                      : IconButton(
                          tooltip: 'Discard changes',
                          onPressed: saving.value ? null : () => close(),
                          icon: const Icon(BuzzIcons.x),
                        ),
                  trailing: defaultTargetPlatform == TargetPlatform.iOS
                      ? IosGlassNavigationButton(
                          icon: IosGlassNavigationIcon.check,
                          semanticLabel: applyLabel,
                          onPressed: onApply,
                          isBusy: saving.value,
                        )
                      : IconButton(
                          tooltip: applyLabel,
                          onPressed: onApply,
                          icon: const Icon(BuzzIcons.check),
                        ),
                ),
                Flexible(
                  child: SingleChildScrollView(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        AnimatedSize(
                          duration: MediaQuery.disableAnimationsOf(context)
                              ? Duration.zero
                              : sectionExpandDuration,
                          alignment: Alignment.topCenter,
                          curve: sectionExpandCurve,
                          child: AppListCard(
                            // A rounded clip around UIKit pickers also clips Flutter's
                            // following overlay, hiding the management actions below.
                            clipBehavior:
                                defaultTargetPlatform == TargetPlatform.iOS
                                ? Clip.none
                                : Clip.antiAlias,
                            dividerIndent: Grid.xs,
                            children: [
                              choices<ChannelSortMode>(
                                'Sort',
                                {
                                  ChannelSortMode.alpha: 'A–Z',
                                  ChannelSortMode.recent: 'Recent',
                                },
                                sort.value,
                                saving.value || (!global && !sortEditable)
                                    ? null
                                    : (value) => sort.value = value,
                              ),
                              choices<ChannelFilterMode>(
                                'View',
                                {
                                  ChannelFilterMode.all: 'All chats',
                                  ChannelFilterMode.unreads: 'Unreads',
                                  ChannelFilterMode.active: 'Active',
                                },
                                mode.value,
                                writable
                                    ? (value) {
                                        filterDirty.value = true;
                                        mode.value = value;
                                      }
                                    : null,
                              ),
                              if (mode.value != ChannelFilterMode.active)
                                choices<int>(
                                  'Show',
                                  {
                                    for (final n in sectionCounts)
                                      n: n == 0 ? 'All' : '$n',
                                  },
                                  count.value,
                                  writable
                                      ? (value) {
                                          filterDirty.value = true;
                                          count.value = value;
                                        }
                                      : null,
                                ),
                              if (mode.value == ChannelFilterMode.active)
                                choices<int>(
                                  'Active in the past',
                                  {
                                    for (final day in {
                                      1,
                                      7,
                                      30,
                                      90,
                                      if (days.value != null) days.value!,
                                    })
                                      day: '$day ${day == 1 ? 'day' : 'days'}',
                                  },
                                  days.value,
                                  writable
                                      ? (value) {
                                          filterDirty.value = true;
                                          days.value = value;
                                        }
                                      : null,
                                ),
                            ],
                          ),
                        ),
                        if (global && canReorder)
                          AppListCard(
                            children: [
                              AppListRow(
                                title: 'Edit Sections',
                                icon: BuzzIcons.pencil,
                                onTap: saving.value
                                    ? null
                                    : () => apply('editSections'),
                              ),
                            ],
                          ),
                        if (!global && manager?.ready != true)
                          const Text(
                            'Filters are read-only until preferences load from the community.',
                          ),
                        if (!global &&
                            manager?.ready != true &&
                            manager?.error != null)
                          TextButton(
                            onPressed: manager?.busy == true
                                ? null
                                : () {
                                    failed.value = false;
                                    manager?.refresh();
                                  },
                            child: const Text('Retry loading preferences'),
                          ),
                        if (!centralized &&
                            (custom && layoutEditable || canReorder))
                          AppListCard(
                            children: [
                              if (custom && layoutEditable)
                                AppListRow(
                                  title: 'Rename section',
                                  icon: BuzzIcons.pencil,
                                  onTap: saving.value
                                      ? null
                                      : () => close('rename'),
                                ),
                              if (canReorder)
                                AppListRow(
                                  title: 'Reorder sections',
                                  icon: BuzzIcons.arrowUpDown,
                                  onTap: saving.value
                                      ? null
                                      : () => close('reorder'),
                                ),
                              if (custom && layoutEditable)
                                AppListRow(
                                  title: 'Delete section',
                                  icon: BuzzIcons.trash2,
                                  titleColor: context.colors.error,
                                  onTap: saving.value
                                      ? null
                                      : () => close('delete'),
                                ),
                            ],
                          ),
                      ],
                    ),
                  ),
                ),
                if (failed.value)
                  Padding(
                    padding: const EdgeInsets.symmetric(
                      horizontal: Grid.gutter,
                      vertical: Grid.xxs,
                    ),
                    child: Semantics(
                      liveRegion: true,
                      child: Text(
                        'Could not confirm the filter save. Your choices are kept; try again.',
                        style: TextStyle(color: context.colors.error),
                      ),
                    ),
                  ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
