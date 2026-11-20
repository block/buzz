part of '../channels_page.dart';

/// A workspace. Edit actions are null where mobile can't write it: always
/// for desktop's unified workspaces, and for legacy Solo workspaces.
class _CustomChannelSection extends StatelessWidget {
  final WorkspaceSection section;
  final List<Channel> channels;
  final bool Function(Channel) isVisible;
  final Widget Function(Channel) buildRow;
  final bool expanded;
  final bool isFirst;
  final bool isLast;
  final VoidCallback onToggle;
  final _SectionEdits? edits;
  final ChannelSortMode sortMode;
  final ValueChanged<ChannelSortMode>? onSortModeChange;

  const _CustomChannelSection({
    super.key,
    required this.section,
    required this.channels,
    required this.isVisible,
    required this.buildRow,
    required this.expanded,
    required this.isFirst,
    required this.isLast,
    required this.onToggle,
    required this.edits,
    required this.sortMode,
    required this.onSortModeChange,
  });

  @override
  Widget build(BuildContext context) => SliverMainAxisGroup(
    slivers: [
      SliverToBoxAdapter(
        child: _CustomSectionHeader(
          section: section,
          expanded: expanded,
          isFirst: isFirst,
          isLast: isLast,
          onToggle: onToggle,
          edits: edits,
          sortMode: sortMode,
          onSortModeChange: onSortModeChange,
        ),
      ),
      _SectionChannelsBody(
        unreadChannelIds: unreadChannelIds,
        attentionChannelIds: attentionChannelIds,
        expanded: expanded,
        channels: channels,
        isVisible: isVisible,
        buildRow: buildRow,
      ),
    ],
  );
}

typedef _SectionEdits = ({
  VoidCallback onRename,
  VoidCallback onDelete,
  VoidCallback onMoveUp,
  VoidCallback onMoveDown,
});

/// A workspace's decorative icon: a Unicode emoji, or a community custom
/// emoji. As on desktop, a shortcode the catalog lacks draws nothing.
class _WorkspaceIcon extends ConsumerWidget {
  const _WorkspaceIcon(this.icon);
  final String icon;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    Widget spaced(Widget child) => Padding(
      padding: const EdgeInsets.only(right: Grid.quarter),
      child: ExcludeSemantics(child: child),
    );
    if (!(icon.length > 2 && icon.startsWith(':') && icon.endsWith(':'))) {
      return spaced(Text(icon, style: navigationSectionTextStyle));
    }
    final shortcode = normalizeShortcode(icon);
    final url = shortcode == null
        ? null
        : ref.watch(
            customEmojiListProvider.select(
              (palette) => palette
                  .where((emoji) => emoji.shortcode == shortcode)
                  .firstOrNull
                  ?.url,
            ),
          );
    if (url == null) return const SizedBox.shrink();
    return spaced(
      CustomEmojiImage(
        shortcode: shortcode!,
        url: url,
        size: _kWorkspaceIconSize,
        compact: true,
      ),
    );
  }
}

const _kWorkspaceIconSize = 16.0;

class _CustomSectionHeader extends StatelessWidget {
  final WorkspaceSection section;
  final bool expanded;
  final bool isFirst;
  final bool isLast;
  final VoidCallback onToggle;
  final _SectionEdits? edits;
  final ChannelSortMode sortMode;
  final ValueChanged<ChannelSortMode>? onSortModeChange;

  const _CustomSectionHeader({
    required this.section,
    required this.expanded,
    required this.isFirst,
    required this.isLast,
    required this.onToggle,
    required this.edits,
    required this.sortMode,
    required this.onSortModeChange,
  });

  @override
  Widget build(BuildContext context) {
    final label = section.name;
    final edits = this.edits;
    final onSortModeChange = this.onSortModeChange;
    Future<void> openOptions(BuildContext buttonContext) async {
      final action = await _showSectionViewSheet(
        buttonContext,
        title: label,
        group: sectionSortGroupKey(section.id),
        sortMode: sortMode,
        onSort: (mode) => onSortModeChange?.call(mode),
        custom: edits != null,
        isFirst: isFirst,
        isLast: isLast,
      );
      if (!buttonContext.mounted || edits == null) return;
      switch (action) {
        case 'rename':
          edits.onRename();
        case 'move_up':
          edits.onMoveUp();
        case 'move_down':
          edits.onMoveDown();
        case 'delete':
          edits.onDelete();
      }
    }

    final menu = [
      if (edits != null) ...[
        (label: 'Rename section', symbol: 'pencil', onPressed: edits.onRename),
        (
          label: 'Move up',
          symbol: 'arrow.up',
          onPressed: isFirst ? null : edits.onMoveUp,
        ),
        (
          label: 'Move down',
          symbol: 'arrow.down',
          onPressed: isLast ? null : edits.onMoveDown,
        ),
      ],
      ..._sortMenu(sortMode, onSortModeChange),
      (
        label: 'Filter…',
        symbol: 'line.3.horizontal.decrease',
        onPressed: () => openOptions(context),
      ),
      if (edits != null)
        (label: 'Delete section', symbol: 'trash', onPressed: edits.onDelete),
    ];
    final sortAt = edits == null ? 0 : 3;

    final sectionColor = navigationHeadingForeground(context);
    return GestureDetector(
      onTap: onToggle,
      behavior: HitTestBehavior.opaque,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          _kSectionHeaderVerticalPadding,
          Grid.gutter,
          _kSectionHeaderVerticalPadding,
        ),
        child: Row(
          children: [
            Expanded(
              // The options button stays its own screen-reader stop.
              child: Semantics(
                label: label,
                button: true,
                expanded: expanded,
                onTap: onToggle,
                excludeSemantics: true,
                child: _SectionLabel(
                  label: label,
                  icon: switch (section.icon) {
                    final icon? => _WorkspaceIcon(icon),
                    null => null,
                  },
                  expanded: expanded,
                  color: sectionColor,
                ),
              ),
            ),
            if (_usesNativeSectionMenu(context))
              IosMessageCreateButton(
                key: ValueKey('section-menu-${section.id}'),
                icon: 'more',
                trailingIconSlotWidth: _kChannelIconSize,
                accessibilityLabel: '$label options',
                width: 44,
                height: 44,
                plain: true,
                iconSize: 14,
                foregroundColor: navigationAccessoryForeground(context),
                selectedIndex: onSortModeChange == null
                    ? null
                    : sortAt + (sortMode == ChannelSortMode.recent ? 0 : 1),
                dividerBefore: {
                  if (edits != null) sortAt,
                  if (edits != null) menu.length - 1,
                },
                destructiveIndices: {if (edits != null) menu.length - 1},
                actions: menu,
              )
            else
              Builder(
                builder: (buttonContext) => _SectionOptionsButton(
                  key: ValueKey('section-menu-${section.id}'),
                  tooltip: '$label options',
                  icon: Icon(
                    BuzzIcons.ellipsis,
                    size: 14,
                    color: navigationAccessoryForeground(context),
                  ),
                  onPressed: () => openOptions(buttonContext),
                ),
              ),
          ],
        ),
      ),
    );
  }
}

class _SectionNameDialog extends HookWidget {
  final String title;
  final String confirmLabel;
  final String initialValue;

  const _SectionNameDialog({
    required this.title,
    required this.confirmLabel,
    this.initialValue = '',
  });

  @override
  Widget build(BuildContext context) {
    final controller = useTextEditingController(text: initialValue);

    void confirm() {
      final name = controller.text.trim();
      if (name.isNotEmpty) Navigator.of(context).pop(name);
    }

    return AlertDialog(
      title: Text(title),
      content: TextField(
        controller: controller,
        autofocus: true,
        decoration: const InputDecoration(labelText: 'Name'),
        onSubmitted: (_) => confirm(),
      ),
      actions: [
        TextButton(
          onPressed: () => Navigator.of(context).pop(),
          child: const Text('Cancel'),
        ),
        TextButton(onPressed: confirm, child: Text(confirmLabel)),
      ],
    );
  }
}

class _ChannelSection extends StatelessWidget {
  final String title;

  /// The sort and filter group key, such as `channels` or `dms`.
  final String group;
  final bool expanded;
  final VoidCallback onToggle;
  final List<Channel> channels;
  final bool Function(Channel) isVisible;
  final Widget Function(Channel) buildRow;
  final String emptyLabel;
  final ChannelSortMode? sortMode;
  final ValueChanged<ChannelSortMode>? onSortModeChange;

  const _ChannelSection({
    super.key,
    required this.title,
    required this.group,
    required this.expanded,
    required this.onToggle,
    required this.channels,
    required this.isVisible,
    required this.buildRow,
    required this.emptyLabel,
    this.sortMode,
    this.onSortModeChange,
  });

  @override
  Widget build(BuildContext context) => SliverMainAxisGroup(
    slivers: [
      SliverToBoxAdapter(
        child: _SectionHeader(
          label: title,
          group: group,
          expanded: expanded,
          onToggle: onToggle,
          sortMode: sortMode,
          onSortModeChange: onSortModeChange,
        ),
      ),
      _SectionChannelsBody(
        unreadChannelIds: unreadChannelIds,
        attentionChannelIds: attentionChannelIds,
        expanded: expanded,
        channels: channels,
        isVisible: isVisible,
        emptySliver: SliverToBoxAdapter(
          child: Padding(
            padding: const EdgeInsets.only(
              left: _kChannelLabelInset,
              right: _kChannelSectionInset,
              top: Grid.half,
              bottom: Grid.half,
            ),
            child: Align(
              alignment: Alignment.centerLeft,
              child: Text(
                emptyLabel,
                style: contentListBodyTextStyle.copyWith(
                  color: context.colors.onSurfaceVariant.withValues(
                    alpha: 0.65,
                  ),
                ),
              ),
            ),
          ),
        ),
        buildRow: buildRow,
      ),
    ],
  );
}

class _SectionChannelsBody extends HookWidget {
  const _SectionChannelsBody({
    required this.expanded,
    required this.channels,
    required this.isVisible,
    required this.buildRow,
    required this.unreadChannelIds,
    required this.attentionChannelIds,
    this.emptySliver,
  });

  final bool expanded;
  final List<Channel> channels;
  final bool Function(Channel) isVisible;
  final Widget Function(Channel) buildRow;
  final Set<String> unreadChannelIds;
  final Set<String> attentionChannelIds;
  final Widget? emptySliver;

  @override
  Widget build(BuildContext context) {
    final visibleChannels = <Channel>[];
    final hiddenIds = <String>[];
    for (final channel in channels) {
      if (isVisible(channel)) {
        visibleChannels.add(channel);
      } else {
        hiddenIds.add(channel.id);
      }
    }
    final hiddenCount = hiddenIds.length;
    final reveal = useState(false);
    useEffect(() {
      reveal.value = false;
      return null;
    }, [hiddenIds.join(',')]);
    final displayed = reveal.value ? channels : visibleChannels;

    final overflow = displayed.skip(LimitedSectionSliver.visibleLimit);
    final activity =
        overflow.any((channel) => attentionChannelIds.contains(channel.id))
        ? 'Needs attention'
        : overflow.any((channel) => unreadChannelIds.contains(channel.id))
        ? 'Unread'
        : null;

    return CollapsibleSliverBody(
      expanded: expanded,
      sliver: channels.isEmpty && emptySliver != null
          ? emptySliver!
          : SliverMainAxisGroup(
              slivers: [
                LimitedSectionSliver(
                  itemCount: displayed.length,
                  revealAll: reveal.value,
                  overflowActivity: activity,
                  findChildIndexCallback: (key) {
                    final index = displayed.indexWhere(
                      (channel) => ValueKey(channel.id) == key,
                    );
                    return index < 0 ? null : index;
                  },
                  itemBuilder: (context, index) => buildRow(displayed[index]),
                ),
                if (hiddenCount > 0)
                  SliverToBoxAdapter(
                    child: Align(
                      alignment: Alignment.centerLeft,
                      child: TextButton(
                        onPressed: () => reveal.value = !reveal.value,
                        child: Text(
                          reveal.value
                              ? 'Hide filtered channels'
                              : 'Show hidden channels · $hiddenCount',
                        ),
                      ),
                    ),
                  ),
              ],
            ),
    );
  }
}

class _EmptyState extends StatelessWidget {
  const _EmptyState();

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: MediaQuery.sizeOf(context).height * 0.55,
      child: Center(
        child: Column(
          mainAxisSize: MainAxisSize.min,
          children: [
            Icon(
              BuzzIcons.messagesSquare,
              size: Grid.xl,
              color: context.colors.onSurfaceVariant,
            ),
            const SizedBox(height: Grid.xs),
            Text(
              'No conversations yet',
              style: context.textTheme.bodyLarge?.copyWith(
                color: context.colors.onSurfaceVariant,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _SectionHeader extends StatelessWidget {
  final String label;
  final String group;
  final bool expanded;
  final VoidCallback onToggle;
  final ChannelSortMode? sortMode;

  /// Null when the sort is read-only; the menu still offers filters.
  final ValueChanged<ChannelSortMode>? onSortModeChange;

  const _SectionHeader({
    required this.label,
    required this.group,
    required this.expanded,
    required this.onToggle,
    this.sortMode,
    this.onSortModeChange,
  });

  @override
  Widget build(BuildContext context) {
    void openOptions(BuildContext buttonContext, ChannelSortMode mode) {
      _showSectionViewSheet(
        buttonContext,
        title: label,
        group: group,
        sortMode: mode,
        onSort: (value) => onSortModeChange?.call(value),
      );
    }

    final sectionColor = navigationHeadingForeground(context);

    return GestureDetector(
      onTap: onToggle,
      behavior: HitTestBehavior.opaque,
      child: Padding(
        padding: const EdgeInsets.fromLTRB(
          Grid.gutter,
          _kSectionHeaderVerticalPadding,
          Grid.gutter,
          _kSectionHeaderVerticalPadding,
        ),
        child: Row(
          children: [
            Expanded(
              child: _SectionLabel(
                label: label,
                expanded: expanded,
                color: sectionColor,
              ),
            ),
            if (sortMode case final mode?) ...[
              if (_usesNativeSectionMenu(context))
                IosMessageCreateButton(
                  key: ValueKey('sort-menu-$label'),
                  icon: 'more',
                  trailingIconSlotWidth: _kChannelIconSize,
                  accessibilityLabel: '$label options',
                  width: 44,
                  height: 44,
                  plain: true,
                  iconSize: 14,
                  foregroundColor: navigationAccessoryForeground(context),
                  selectedIndex: onSortModeChange == null
                      ? null
                      : mode == ChannelSortMode.recent
                      ? 0
                      : 1,
                  actions: [
                    ..._sortMenu(mode, onSortModeChange),
                    (
                      label: 'Filter…',
                      symbol: 'line.3.horizontal.decrease',
                      onPressed: () => openOptions(context, mode),
                    ),
                  ],
                )
              else
                Builder(
                  builder: (buttonContext) => _SectionOptionsButton(
                    key: ValueKey('sort-menu-$label'),
                    tooltip: '$label options',
                    icon: Icon(
                      BuzzIcons.ellipsis,
                      size: 14,
                      color: navigationAccessoryForeground(context),
                    ),
                    onPressed: () => openOptions(buttonContext, mode),
                  ),
                ),
            ],
          ],
        ),
      ),
    );
  }
}

class _SectionLabel extends StatelessWidget {
  const _SectionLabel({
    required this.label,
    this.icon,
    required this.expanded,
    required this.color,
  });

  final String label;

  /// A decorative icon before the label.
  final Widget? icon;
  final bool expanded;
  final Color color;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: Grid.xxs),
    child: Row(
      children: [
        ?icon,
        Flexible(
          child: Text(
            label,
            maxLines: 1,
            overflow: TextOverflow.ellipsis,
            style: navigationSectionTextStyle.copyWith(
              color: color,
              fontWeight: FontWeight.w500,
            ),
          ),
        ),
        const SizedBox(width: Grid.quarter),
        _SectionChevron(
          expanded: expanded,
          color: navigationAccessoryForeground(context),
        ),
      ],
    ),
  );
}

class _SectionChevron extends StatelessWidget {
  final bool expanded;
  final Color color;

  const _SectionChevron({required this.expanded, required this.color});

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.of(context).disableAnimations;

    return AnimatedRotation(
      turns: expanded ? 0 : -0.25,
      duration: reducedMotion ? Duration.zero : sectionExpandDuration,
      curve: sectionExpandCurve,
      child: Icon(BuzzIcons.chevronDown, size: 14, color: color),
    );
  }
}

/// Sort choices for a section menu; none while the sort is read-only.
List<({String label, String symbol, VoidCallback? onPressed})> _sortMenu(
  ChannelSortMode mode,
  ValueChanged<ChannelSortMode>? onChange,
) => onChange == null
    ? const []
    : [
        (
          label: 'Sort: Recent',
          symbol: '',
          onPressed: () => onChange(ChannelSortMode.recent),
        ),
        (
          label: 'Sort: A–Z',
          symbol: '',
          onPressed: () => onChange(ChannelSortMode.alpha),
        ),
      ];

bool _usesNativeSectionMenu(BuildContext context) =>
    defaultTargetPlatform == TargetPlatform.iOS &&
    (DuoToolbarScope.maybeStripOf(context) ?? FoldGeometry.of(context).strip) !=
        null;

class _SectionOptionsButton extends StatelessWidget {
  const _SectionOptionsButton({
    super.key,
    required this.tooltip,
    required this.icon,
    required this.onPressed,
  });

  final String tooltip;
  final Widget icon;
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) => Semantics(
    button: true,
    label: tooltip,
    onTap: onPressed,
    excludeSemantics: true,
    child: Tooltip(
      message: tooltip,
      child: SizedBox.square(
        dimension: 44,
        child: InkWell(
          onTap: onPressed,
          borderRadius: BorderRadius.circular(Radii.full),
          child: Align(
            alignment: Alignment.centerRight,
            child: SizedBox(
              width: _kChannelIconSize,
              child: Center(child: icon),
            ),
          ),
        ),
      ),
    ),
  );
}
