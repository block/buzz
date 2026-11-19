part of '../channels_page.dart';

class _CustomChannelSection extends StatelessWidget {
  final ChannelSection section;
  final List<Channel> channels;
  final bool Function(Channel) isVisible;
  final Set<String> unreadChannelIds;
  final Set<String> attentionChannelIds;
  final Set<String> mutedChannelIds;
  final String? currentPubkey;
  final bool expanded;
  final bool isFirst;
  final bool isLast;
  final VoidCallback onToggle;
  final VoidCallback onRename;
  final VoidCallback onDelete;
  final VoidCallback onMoveUp;
  final VoidCallback onMoveDown;
  final ChannelSortMode sortMode;
  final ValueChanged<ChannelSortMode> onSortModeChange;
  final Future<void> Function(Channel channel) onSelectChannel;

  const _CustomChannelSection({
    super.key,
    required this.section,
    required this.channels,
    required this.isVisible,
    required this.unreadChannelIds,
    required this.attentionChannelIds,
    required this.mutedChannelIds,
    required this.currentPubkey,
    required this.expanded,
    required this.isFirst,
    required this.isLast,
    required this.onToggle,
    required this.onRename,
    required this.onDelete,
    required this.onMoveUp,
    required this.onMoveDown,
    required this.sortMode,
    required this.onSortModeChange,
    required this.onSelectChannel,
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
          onRename: onRename,
          onDelete: onDelete,
          onMoveUp: onMoveUp,
          onMoveDown: onMoveDown,
          sortMode: sortMode,
          onSortModeChange: onSortModeChange,
        ),
      ),
      _SectionChannelsBody(
        expanded: expanded,
        channels: channels,
        isVisible: isVisible,
        buildRow: (channel) => _ChannelTile(
          key: ValueKey(channel.id),
          channel: channel,
          isUnread: unreadChannelIds.contains(channel.id),
          hasAttention: attentionChannelIds.contains(channel.id),
          isMuted: mutedChannelIds.contains(channel.id),
          currentPubkey: currentPubkey,
          onTap: () => onSelectChannel(channel),
          sectionId: section.id,
        ),
      ),
    ],
  );
}

class _CustomSectionHeader extends StatelessWidget {
  final ChannelSection section;
  final bool expanded;
  final bool isFirst;
  final bool isLast;
  final VoidCallback onToggle;
  final VoidCallback onRename;
  final VoidCallback onDelete;
  final VoidCallback onMoveUp;
  final VoidCallback onMoveDown;
  final ChannelSortMode sortMode;
  final ValueChanged<ChannelSortMode> onSortModeChange;

  const _CustomSectionHeader({
    required this.section,
    required this.expanded,
    required this.isFirst,
    required this.isLast,
    required this.onToggle,
    required this.onRename,
    required this.onDelete,
    required this.onMoveUp,
    required this.onMoveDown,
    required this.sortMode,
    required this.onSortModeChange,
  });

  @override
  Widget build(BuildContext context) {
    Future<void> openOptions(BuildContext buttonContext) async {
      final action = await _showSectionViewSheet(
        buttonContext,
        title: section.name,
        group: sectionSortGroupKey(section.id),
        sortMode: sortMode,
        onSort: onSortModeChange,
        custom: true,
        isFirst: isFirst,
        isLast: isLast,
      );
      if (!buttonContext.mounted) return;
      switch (action) {
        case 'rename':
          onRename();
        case 'move_up':
          onMoveUp();
        case 'move_down':
          onMoveDown();
        case 'delete':
          onDelete();
      }
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
                label: section.name,
                expanded: expanded,
                color: sectionColor,
              ),
            ),
            if (_usesNativeSectionMenu(context))
              IosMessageCreateButton(
                key: ValueKey('section-menu-${section.id}'),
                icon: 'more',
                trailingIconSlotWidth: _kChannelIconSize,
                accessibilityLabel: '${section.name} options',
                width: 44,
                height: 44,
                plain: true,
                iconSize: 14,
                foregroundColor: navigationAccessoryForeground(context),
                selectedIndex: sortMode == ChannelSortMode.recent ? 3 : 4,
                dividerBefore: const {3, 6},
                destructiveIndices: const {6},
                actions: [
                  (
                    label: 'Rename section',
                    symbol: 'pencil',
                    onPressed: onRename,
                  ),
                  (
                    label: 'Move up',
                    symbol: 'arrow.up',
                    onPressed: isFirst ? null : onMoveUp,
                  ),
                  (
                    label: 'Move down',
                    symbol: 'arrow.down',
                    onPressed: isLast ? null : onMoveDown,
                  ),
                  (
                    label: 'Sort: Recent',
                    symbol: '',
                    onPressed: () => onSortModeChange(ChannelSortMode.recent),
                  ),
                  (
                    label: 'Sort: A–Z',
                    symbol: '',
                    onPressed: () => onSortModeChange(ChannelSortMode.alpha),
                  ),
                  (
                    label: 'Filter…',
                    symbol: 'line.3.horizontal.decrease',
                    onPressed: () => openOptions(context),
                  ),
                  (
                    label: 'Delete section',
                    symbol: 'trash',
                    onPressed: onDelete,
                  ),
                ],
              )
            else
              Builder(
                builder: (buttonContext) => _SectionOptionsButton(
                  key: ValueKey('section-menu-${section.id}'),
                  tooltip: '${section.name} options',
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
  final bool expanded;
  final VoidCallback onToggle;
  final List<Channel> channels;
  final bool Function(Channel) isVisible;
  final Set<String> unreadChannelIds;
  final Set<String> attentionChannelIds;
  final Set<String> mutedChannelIds;
  final String? currentPubkey;
  final String emptyLabel;
  final ChannelSortMode? sortMode;
  final ValueChanged<ChannelSortMode>? onSortModeChange;
  final Future<void> Function(Channel channel) onSelectChannel;

  const _ChannelSection({
    super.key,
    required this.title,
    required this.expanded,
    required this.onToggle,
    required this.channels,
    required this.isVisible,
    required this.unreadChannelIds,
    required this.attentionChannelIds,
    required this.mutedChannelIds,
    required this.currentPubkey,
    required this.emptyLabel,
    this.sortMode,
    this.onSortModeChange,
    required this.onSelectChannel,
  });

  @override
  Widget build(BuildContext context) => SliverMainAxisGroup(
    slivers: [
      SliverToBoxAdapter(
        child: _SectionHeader(
          label: title,
          expanded: expanded,
          onToggle: onToggle,
          sortMode: sortMode,
          onSortModeChange: onSortModeChange,
        ),
      ),
      _SectionChannelsBody(
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
        buildRow: (channel) => _ChannelTile(
          key: ValueKey(channel.id),
          channel: channel,
          isUnread: unreadChannelIds.contains(channel.id),
          hasAttention: attentionChannelIds.contains(channel.id),
          isMuted: mutedChannelIds.contains(channel.id),
          currentPubkey: currentPubkey,
          onTap: () => onSelectChannel(channel),
          sectionId: null,
        ),
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
    this.emptySliver,
  });

  final bool expanded;
  final List<Channel> channels;
  final bool Function(Channel) isVisible;
  final Widget Function(Channel) buildRow;
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

    return CollapsibleSliverBody(
      expanded: expanded,
      sliver: channels.isEmpty && emptySliver != null
          ? emptySliver!
          : SliverList.builder(
              itemCount: displayed.length + (hiddenCount == 0 ? 0 : 1),
              findChildIndexCallback: (key) {
                final index = displayed.indexWhere(
                  (c) => ValueKey(c.id) == key,
                );
                return index < 0 ? null : index;
              },
              itemBuilder: (context, index) {
                if (index < displayed.length) return buildRow(displayed[index]);
                return Align(
                  alignment: Alignment.centerLeft,
                  child: TextButton(
                    onPressed: () => reveal.value = !reveal.value,
                    child: Text(
                      reveal.value
                          ? 'Hide filtered channels'
                          : 'Show hidden channels · $hiddenCount',
                    ),
                  ),
                );
              },
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
  final bool expanded;
  final VoidCallback onToggle;
  final ChannelSortMode? sortMode;
  final ValueChanged<ChannelSortMode>? onSortModeChange;

  const _SectionHeader({
    required this.label,
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
        group: label.toLowerCase(),
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
                  selectedIndex: mode == ChannelSortMode.recent ? 0 : 1,
                  actions: [
                    (
                      label: 'Sort: Recent',
                      symbol: '',
                      onPressed: () =>
                          onSortModeChange?.call(ChannelSortMode.recent),
                    ),
                    (
                      label: 'Sort: A–Z',
                      symbol: '',
                      onPressed: () =>
                          onSortModeChange?.call(ChannelSortMode.alpha),
                    ),
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
    required this.expanded,
    required this.color,
  });

  final String label;
  final bool expanded;
  final Color color;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: Grid.xxs),
    child: Row(
      children: [
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
