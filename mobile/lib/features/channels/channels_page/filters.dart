part of '../channels_page.dart';

Future<String?> _showSectionViewSheet(
  BuildContext context, {
  required String title,
  required String group,
  required ChannelSortMode sortMode,
  required ValueChanged<ChannelSortMode> onSort,
  bool custom = false,
  bool isFirst = false,
  bool isLast = false,
}) => showBuzzModalBottomSheet<String>(
  context: context,
  showCloseButton: false,
  showDragHandle: false,
  isScrollControlled: true,
  isDismissible: false,
  enableDrag: false,
  builder: (_) => _SectionViewSheet(
    title: title,
    group: group,
    initialSort: sortMode,
    onSort: onSort,
    custom: custom,
    isFirst: isFirst,
    isLast: isLast,
  ),
);

class _SectionViewSheet extends HookConsumerWidget {
  final String title;
  final String group;
  final ChannelSortMode initialSort;
  final ValueChanged<ChannelSortMode> onSort;
  final bool custom;
  final bool isFirst;
  final bool isLast;

  const _SectionViewSheet({
    required this.title,
    required this.group,
    required this.initialSort,
    required this.onSort,
    required this.custom,
    required this.isFirst,
    required this.isLast,
  });

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final manager = ref.watch(channelFilterProvider);
    useListenable(manager);
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

    Future<void> apply() async {
      saving.value = true;
      failed.value = false;
      try {
        final filterChanged =
            mode.value !=
                (manager?.store.modeFor(group) ?? ChannelFilterMode.all) ||
            (mode.value == ChannelFilterMode.active &&
                days.value * 86400 != manager?.store.windowFor(group));
        if (filterDirty.value && filterChanged) {
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
        if (sort.value != initialSort) onSort(sort.value);
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
    ) => AppListCard(
      label: label,
      dividerIndent: Grid.xs,
      children: [
        for (final entry in values.entries)
          Semantics(
            checked: entry.key == selected,
            inMutuallyExclusiveGroup: true,
            enabled: change != null,
            child: AppListRow(
              key: ValueKey('section-choice-$label-${entry.value}'),
              title: entry.value,
              trailing: entry.key == selected
                  ? Icon(
                      BuzzIcons.check,
                      size: 18,
                      color: context.colors.primary,
                    )
                  : null,
              titleColor: change == null
                  ? context.colors.onSurfaceVariant
                  : null,
              onTap: change == null
                  ? null
                  : () {
                      change(entry.key);
                    },
            ),
          ),
      ],
    );
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
                  trailing: saving.value
                      ? const IconButton(
                          tooltip: 'Saving…',
                          onPressed: null,
                          icon: Icon(BuzzIcons.x),
                        )
                      : null,
                ),
                Flexible(
                  child: SingleChildScrollView(
                    child: Column(
                      crossAxisAlignment: CrossAxisAlignment.start,
                      children: [
                        choices<ChannelSortMode>(
                          'Sort',
                          {
                            ChannelSortMode.alpha: 'A–Z',
                            ChannelSortMode.recent: 'Recent',
                          },
                          sort.value,
                          saving.value ? null : (value) => sort.value = value,
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
                              for (final day in {1, 7, 30, 90, days.value})
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
                        if (manager?.ready != true)
                          const Text(
                            'Filters are read-only until preferences load from the community.',
                          ),
                        if (manager?.ready != true &&
                            manager?.error != null &&
                            !failed.value)
                          TextButton(
                            onPressed: manager?.busy == true
                                ? null
                                : manager?.refresh,
                            child: const Text('Retry loading preferences'),
                          ),
                        if (custom) ...[
                          const Divider(),
                          for (final action in {
                            'rename': 'Rename section',
                            'move_up': 'Move up',
                            'move_down': 'Move down',
                            'delete': 'Delete section',
                          }.entries)
                            ListTile(
                              contentPadding: EdgeInsets.zero,
                              title: Text(
                                action.value,
                                style: action.key == 'delete'
                                    ? TextStyle(color: context.colors.error)
                                    : null,
                              ),
                              enabled:
                                  !saving.value &&
                                  !(action.key == 'move_up' && isFirst) &&
                                  !(action.key == 'move_down' && isLast),
                              onTap: () => close(action.key),
                            ),
                        ],
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
                const SizedBox(height: Grid.xxs),
                Padding(
                  padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
                  child: Align(
                    alignment: Alignment.centerRight,
                    child: Wrap(
                      alignment: WrapAlignment.end,
                      spacing: Grid.xxs,
                      runSpacing: Grid.xxs,
                      children: [
                        TextButton(
                          onPressed: saving.value ? null : () => close(),
                          child: const Text('Cancel'),
                        ),
                        FilledButton(
                          style: FilledButton.styleFrom(
                            backgroundColor: context.colors.onSurface,
                            foregroundColor: context.colors.surface,
                          ),
                          onPressed: saving.value || manager?.busy == true
                              ? null
                              : apply,
                          child: Text(
                            saving.value
                                ? 'Saving…'
                                : failed.value
                                ? 'Retry'
                                : 'Apply',
                          ),
                        ),
                      ],
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
