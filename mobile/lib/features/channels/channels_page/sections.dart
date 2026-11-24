part of '../channels_page.dart';

/// A workspace. Edit actions are null where mobile can't write it: always
/// for desktop's unified workspaces, and for legacy Solo workspaces.
class _CustomChannelSection extends StatelessWidget {
  final WorkspaceSection section;
  final List<Channel> channels;
  final bool Function(Channel) isVisible;
  final SectionChatRowBuilder buildRow;
  final Widget Function(Channel) buildDetail;
  final int? previewLimit;
  final bool expanded;
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
    required this.buildDetail,
    required this.previewLimit,
    required this.expanded,
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
          onToggle: onToggle,
          edits: edits,
          sortMode: sortMode,
          onSortModeChange: onSortModeChange,
        ),
      ),
      _SectionChannelsBody(
        sectionKey: 'group:${section.id}',
        title: section.name,
        buildDetail: buildDetail,
        previewLimit: previewLimit,
        expanded: expanded,
        channels: channels,
        isVisible: isVisible,
        buildRow: buildRow,
      ),
    ],
  );
}

typedef _SectionEdits = ({
  Future<void> Function() onRename,
  Future<void> Function() onDelete,
});

/// A workspace's decorative icon: a Unicode emoji, or a community custom
/// emoji. As on desktop, a shortcode the catalog lacks draws nothing.
class _WorkspaceIcon extends ConsumerWidget {
  const _WorkspaceIcon(this.icon, {this.inHeader = false});
  final bool inHeader;
  final String icon;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    Widget display(Widget child, [String? source]) {
      if (!inHeader) return ExcludeSemantics(child: child);
      return _SectionLeadingIcon(child, key: ValueKey((icon, source)));
    }

    if (!(icon.length > 2 && icon.startsWith(':') && icon.endsWith(':'))) {
      return display(
        FittedBox(
          fit: BoxFit.scaleDown,
          child: Text(
            icon.trim(),
            textAlign: TextAlign.center,
            style: const TextStyle(fontSize: _kWorkspaceIconSize, height: 1),
          ),
        ),
      );
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
    if (url == null) return display(const SizedBox.shrink());
    return display(
      CustomEmojiImage(
        shortcode: shortcode!,
        url: url,
        size: inHeader ? _kChannelIconSize : _kWorkspaceIconSize,
        compact: true,
      ),
      url,
    );
  }
}

const _kWorkspaceIconSize = 16.0;

class _SectionLeadingIcon extends HookWidget {
  const _SectionLeadingIcon(this.child, {super.key});
  final Widget child;

  @override
  Widget build(BuildContext context) {
    final shift = useState(Offset.zero);
    return Padding(
      padding: const EdgeInsets.only(right: Grid.inventoryLabelGap),
      child: SizedBox.square(
        dimension: _kChannelIconSize,
        child: Transform.translate(
          offset: shift.value,
          child: _EmojiArtwork(
            onCenter: (value) {
              if (context.mounted) shift.value = value;
            },
            child: ExcludeSemantics(child: child),
          ),
        ),
      ),
    );
  }
}

// Font side bearings and transparent image margins differ between emoji.
// Measure only the painted artwork in the small header icon.
class _EmojiArtwork extends SingleChildRenderObjectWidget {
  const _EmojiArtwork({required this.onCenter, required super.child});
  final ValueChanged<Offset> onCenter;

  @override
  _EmojiArtworkBoundary createRenderObject(BuildContext context) =>
      _EmojiArtworkBoundary(onCenter);

  @override
  void updateRenderObject(BuildContext context, _EmojiArtworkBoundary object) {
    object.onCenter = onCenter;
  }
}

class _EmojiArtworkBoundary extends RenderRepaintBoundary {
  _EmojiArtworkBoundary(this.onCenter);
  ValueChanged<Offset> onCenter;
  int revision = 0;
  bool measuring = false;
  bool measured = false;

  @override
  void paint(PaintingContext context, Offset offset) {
    super.paint(context, offset);
    revision++;
    if (measuring || measured) return;
    measuring = true;
    WidgetsBinding.instance.addPostFrameCallback((_) async {
      try {
        if (!attached || size.isEmpty) return;
        final version = revision;
        const scale = 3.0;
        final image = await toImage(pixelRatio: scale);
        try {
          final pixels = await image.toByteData(
            format: ImageByteFormat.rawRgba,
          );
          if (!attached || pixels == null) return;
          var left = image.width;
          var top = image.height;
          var right = -1;
          var bottom = -1;
          for (var y = 0; y < image.height; y++) {
            for (var x = 0; x < image.width; x++) {
              if (pixels.getUint8(4 * (y * image.width + x) + 3) < 16) continue;
              left = min(left, x);
              right = max(right, x);
              top = min(top, y);
              bottom = max(bottom, y);
            }
          }
          if (right >= left) {
            measured = true;
            onCenter(
              Offset(
                (image.width - left - right - 1) / (2 * scale),
                (image.height - top - bottom - 1) / (2 * scale),
              ),
            );
          }
          if (version != revision) markNeedsPaint();
        } finally {
          image.dispose();
        }
      } finally {
        measuring = false;
      }
    });
  }
}

final _sectionEmoji = RegExp(
  // The VM supports Unicode properties that the analyzer regex parser omits.
  // ignore: valid_regexps
  r'\p{Extended_Pictographic}|\p{Regional_Indicator}|\u20E3',
  unicode: true,
);

class _CustomSectionHeader extends StatelessWidget {
  final WorkspaceSection section;
  final bool expanded;
  final VoidCallback onToggle;
  final _SectionEdits? edits;
  final ChannelSortMode sortMode;
  final ValueChanged<ChannelSortMode>? onSortModeChange;

  const _CustomSectionHeader({
    required this.section,
    required this.expanded,
    required this.onToggle,
    required this.edits,
    required this.sortMode,
    required this.onSortModeChange,
  });

  @override
  Widget build(BuildContext context) {
    final label = section.name;
    // Older section names can include their emoji as the first grapheme.
    final first = label.characters.firstOrNull;
    final hasPrefix =
        section.icon?.trim().isNotEmpty != true &&
        first != null &&
        _sectionEmoji.hasMatch(first) &&
        label.length > first.length;
    final sectionIcon = hasPrefix ? first : section.icon?.trim();
    final edits = this.edits;
    final onSortModeChange = this.onSortModeChange;
    Future<void> openOptions(BuildContext buttonContext) async {
      final action = await showSectionViewSheet(
        buttonContext,
        title: label,
        group: sectionSortGroupKey(section.id),
        sortMode: sortMode,
        onSort: (mode) => onSortModeChange?.call(mode),
        custom: edits != null,
      );
      if (!buttonContext.mounted || edits == null) return;
      switch (action) {
        case 'rename':
          edits.onRename();
        case 'delete':
          edits.onDelete();
      }
    }

    final messages = MessagesSectionOptions.active(context);
    final inset = messages
        ? Grid.inventoryRowInset
        : _kSectionHeaderVerticalPadding;
    return GestureDetector(
      onTap: onToggle,
      behavior: HitTestBehavior.opaque,
      child: Padding(
        padding: EdgeInsets.fromLTRB(
          MessagesSectionOptions.leadingInset(context),
          messages ? Grid.inventorySectionTopInset : inset,
          Grid.twelve,
          inset,
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            Expanded(
              // The options button stays its own screen-reader stop.
              child: Semantics(
                label: label,
                button: true,
                expanded: expanded,
                onTap: onToggle,
                excludeSemantics: true,
                child: SectionHeadingLabel(
                  expanded: expanded,
                  label: hasPrefix
                      ? label.substring(first.length).trimLeft()
                      : label,
                  icon: switch (sectionIcon) {
                    final icon? when icon.trim().isNotEmpty => _WorkspaceIcon(
                      icon,
                      inHeader: true,
                    ),
                    _ => null,
                  },
                ),
              ),
            ),
            if (!messages)
              Builder(
                builder: (buttonContext) => SectionOptionsButton(
                  key: ValueKey('section-menu-${section.id}'),
                  tooltip: '$label options',
                  onPressed: () => openOptions(buttonContext),
                ),
              ),
          ],
        ),
      ),
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
  final SectionChatRowBuilder buildRow;
  final Widget Function(Channel) buildDetail;
  final int? previewLimit;
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
    required this.buildDetail,
    required this.previewLimit,
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
        sectionKey: group,
        title: title,
        buildDetail: buildDetail,
        previewLimit: previewLimit,
        expanded: expanded,
        channels: channels,
        isVisible: isVisible,
        emptySliver: SliverToBoxAdapter(
          child: Padding(
            padding: EdgeInsets.only(
              left: MessagesSectionOptions.leadingInset(context),
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

class _SectionChannelsBody extends StatelessWidget {
  final String title;
  final String sectionKey;
  const _SectionChannelsBody({
    required this.sectionKey,
    required this.title,
    required this.expanded,
    required this.channels,
    required this.isVisible,
    required this.buildRow,
    required this.buildDetail,
    required this.previewLimit,
    this.emptySliver,
  });

  final bool expanded;
  final List<Channel> channels;
  final bool Function(Channel) isVisible;
  final SectionChatRowBuilder buildRow;
  final Widget Function(Channel) buildDetail;
  final int? previewLimit;
  final Widget? emptySliver;

  @override
  Widget build(BuildContext context) {
    final messages = MessagesSectionOptions.active(context);
    final matching = channels.where(isVisible).toList();
    final displayed = matching.take(previewLimit ?? matching.length).toList();
    final hiddenCount = channels.length - displayed.length;
    return CollapsibleSliverBody(
      expanded: expanded,
      sliver: channels.isEmpty && emptySliver != null
          ? emptySliver!
          : SliverList.builder(
              itemCount: displayed.length + (hiddenCount == 0 ? 0 : 1),
              findChildIndexCallback: (key) {
                final index = displayed.indexWhere(
                  (c) => key is ValueKey<String> && c.id == key.value,
                );
                return index < 0 ? null : index;
              },
              itemBuilder: (context, index) {
                if (index < displayed.length) {
                  return buildRow(displayed[index]);
                }
                return SectionShowMore(
                  liveContent: (ref) =>
                      _watchSectionChats(ref, sectionKey, title, messages),
                  leadingInset: MessagesSectionOptions.leadingInset(context),
                  buildRow: buildRow,
                  title: title,
                  channels: channels,
                  buildDetail: buildDetail,
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
      showSectionViewSheet(
        buttonContext,
        title: label,
        group: group,
        sortMode: mode,
        onSort: (value) => onSortModeChange?.call(value),
      );
    }

    final messages = MessagesSectionOptions.active(context);
    final inset = messages
        ? Grid.inventoryRowInset
        : _kSectionHeaderVerticalPadding;
    return GestureDetector(
      onTap: onToggle,
      behavior: HitTestBehavior.opaque,
      child: Padding(
        padding: EdgeInsets.fromLTRB(
          MessagesSectionOptions.leadingInset(context),
          messages ? Grid.inventorySectionTopInset : inset,
          Grid.twelve,
          inset,
        ),
        child: Row(
          crossAxisAlignment: CrossAxisAlignment.center,
          children: [
            Expanded(
              child: Semantics(
                button: true,
                expanded: expanded,
                label: label,
                excludeSemantics: true,
                onTap: onToggle,
                child: SectionHeadingLabel(label: label, expanded: expanded),
              ),
            ),
            if (sortMode case final mode? when !messages) ...[
              Builder(
                builder: (buttonContext) => SectionOptionsButton(
                  key: ValueKey('sort-menu-$label'),
                  tooltip: '$label options',
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

class _MessagesOptionsButton extends StatelessWidget {
  const _MessagesOptionsButton({required this.onPressed});
  final VoidCallback onPressed;

  @override
  Widget build(BuildContext context) {
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      return IosGlassNavigationButton(
        icon: IosGlassNavigationIcon.more,
        semanticLabel: 'Messages options',
        controlSize: buzzNavigationActionSize,
        onPressed: onPressed,
      );
    }
    return IconButton(
      tooltip: 'Messages options',
      icon: const Icon(BuzzIcons.ellipsis),
      onPressed: onPressed,
    );
  }
}
