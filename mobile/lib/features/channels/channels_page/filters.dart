import '../channel_sort/channel_sort_provider.dart';
import '../channel_sections/section_reorder_sheet.dart';
import '../../../shared/widgets/app_list.dart';
import '../channel_sections/channel_sections_provider.dart';
import 'package:flutter/material.dart';
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

Future<String?> showSectionViewSheet(
  BuildContext context, {
  required String title,
  required String group,
  required ChannelSortMode sortMode,
  required ValueChanged<ChannelSortMode> onSort,
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
  try {
    return await showBuzzModalBottomSheet<String>(
      context: context,
      nativeIosBarrier: presentingRoute != null,
      showCloseButton: false,
      showDragHandle: false,
      isScrollControlled: true,
      isDismissible: true,
      enableDrag: false,
      builder: (sheetContext) {
        if (release != null) sheetRoute = ModalRoute.of(sheetContext);
        return _SectionViewSheet(
          onReorder: orderScope != null
              ? () {
                  if (!orderScope.mounted) return;
                  (orderScope.widget as SectionOrderScope).show(sheetContext);
                }
              : null,
          title: title,
          group: group,
          initialSort: sortMode,
          onSort: onSort,
          custom: custom,
        );
      },
    );
  } finally {
    // A pop resolves before the scrim finishes fading.
    if (release != null) {
      await sheetRoute?.completed;
      release();
    }
  }
}

class _SectionViewSheet extends HookConsumerWidget {
  final VoidCallback? onReorder;
  final String title;
  final String group;
  final ChannelSortMode initialSort;
  final ValueChanged<ChannelSortMode> onSort;
  final bool custom;

  const _SectionViewSheet({
    required this.onReorder,
    required this.title,
    required this.group,
    required this.initialSort,
    required this.onSort,
    required this.custom,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final manager = ref.watch(
      group.startsWith('solo:') ? soloFilterProvider : channelFilterProvider,
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
    final sort = useState(initialSort);
    final mode = useState(
      manager?.store.modeFor(group) ?? ChannelFilterMode.all,
    );
    final days = useState(
      (manager?.store.windowFor(group) ?? defaultActiveWindowSeconds) ~/ 86400,
    );
    final filterDirty = useState(false);
    final saving = useState(false);
    final failed = useState(false);
    // A confirmed initial load may arrive while the sheet is open. Preserve edits.
    useEffect(() {
      if (!filterDirty.value && !saving.value) {
        mode.value = manager?.store.modeFor(group) ?? ChannelFilterMode.all;
        days.value =
            (manager?.store.windowFor(group) ?? defaultActiveWindowSeconds) ~/
            86400;
      }
      return null;
    }, [manager, manager?.ready, manager?.store]);
    if (!identical(originalManager.value, manager)) {
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
        manager?.ready == true && manager?.busy == false && !saving.value;
    void close([String? action]) {
      if (!saving.value) Navigator.of(context).pop(action);
    }

    bool filterChanged() =>
        filterDirty.value &&
        (mode.value !=
                (manager?.store.modeFor(group) ?? ChannelFilterMode.all) ||
            (mode.value == ChannelFilterMode.active &&
                days.value * 86400 != manager?.store.windowFor(group)));

    Future<void> apply() async {
      saving.value = true;
      failed.value = false;
      try {
        if (filterChanged()) {
          if (manager == null || !manager.ready || manager.busy) {
            throw StateError('Preferences are read-only');
          }
          await manager.save(
            group,
            mode.value,
            windowSeconds: mode.value == ChannelFilterMode.active
                ? days.value * 86400
                : null,
          );
        }
        if (!context.mounted) return;
        if (sortEditable && sort.value != initialSort) onSort(sort.value);
        Navigator.of(context).pop();
      } catch (_) {
        if (context.mounted) failed.value = true;
      } finally {
        if (context.mounted) saving.value = false;
      }
    }

    Widget choices<T>(
      String label,
      Map<T, String> values,
      T selected,
      ValueChanged<T>? change,
    ) => BuzzSettingMenu<T>(
      controlKey: ValueKey('section-menu-$label'),
      grouped: true,
      trailingIcon: Icons.unfold_more,
      enabled: change != null,
      label: label,
      value: selected,
      valueLabel: values[selected]!,
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
                                saving.value || !sortEditable
                                    ? null
                                    : (value) => sort.value = value,
                              ),
                              choices<ChannelFilterMode>(
                                'Show',
                                {
                                  ChannelFilterMode.all: 'All',
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
                              if (mode.value == ChannelFilterMode.active)
                                choices<int>(
                                  'Active in the past',
                                  {
                                    for (final day in {
                                      1,
                                      7,
                                      30,
                                      90,
                                      days.value,
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
                        if (manager?.ready != true)
                          const Text(
                            'Filters are read-only until preferences load from the community.',
                          ),
                        if (manager?.ready != true && manager?.error != null)
                          TextButton(
                            onPressed: manager?.busy == true
                                ? null
                                : () {
                                    failed.value = false;
                                    manager?.refresh();
                                  },
                            child: const Text('Retry loading preferences'),
                          ),
                        if (custom && layoutEditable || onReorder != null)
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
                              if (onReorder != null)
                                AppListRow(
                                  title: 'Reorder sections',
                                  icon: BuzzIcons.arrowUpDown,
                                  onTap: saving.value ? null : onReorder,
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
