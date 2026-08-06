part of '../emoji_picker.dart';

/// Emoji per row. Matches desktop's `perLine={8}`.
const _emojiPerLine = 8;

/// Rendered glyph size. Standard and custom emoji share this so a community's
/// own emoji sit in the grid at the same weight as the native set.
const _emojiGlyphSize = 28.0;

/// Height of one grid cell. Fixed rather than derived from the cell width so
/// section offsets are exact — [_EmojiSection.extent] is what the rail scrolls
/// to, and a width-dependent row height would make it a guess.
const _cellExtent = 40.0;

/// Height of a pinned section header, padding included. Also fixed, for the
/// same reason.
const _sectionHeaderExtent = 28.0;

/// Cells divide the content width evenly with no gaps, the way emoji-mart lays
/// its grid out; the space around each glyph comes from the cell being larger
/// than the glyph.
SliverGridDelegate get _gridDelegate =>
    const SliverGridDelegateWithFixedCrossAxisCount(
      crossAxisCount: _emojiPerLine,
      mainAxisExtent: _cellExtent,
    );

/// One labelled run of tiles in the continuous grid.
///
/// Sections know their own scroll extent so the rail can jump to them without
/// the grid having been laid out — with ~1.9k emoji most sections are far
/// off-screen, so there is no render object to measure or `ensureVisible`.
class _EmojiSection {
  final String id;
  final String label;
  final IconData icon;
  final int itemCount;
  final Widget Function(BuildContext context, int index) itemBuilder;

  const _EmojiSection({
    required this.id,
    required this.label,
    required this.icon,
    required this.itemCount,
    required this.itemBuilder,
  });

  int get rowCount => (itemCount / _emojiPerLine).ceil();

  double get extent => _sectionHeaderExtent + rowCount * _cellExtent;
}

/// Offset of each section's header, in scroll order. Element `i` is where the
/// rail should land for section `i`.
List<double> _sectionOffsets(List<_EmojiSection> sections) {
  final offsets = <double>[];
  var running = 0.0;
  for (final section in sections) {
    offsets.add(running);
    running += section.extent;
  }
  return offsets;
}

/// Which section owns [offset] — the one whose header is pinned right now.
int _activeSectionIndex(List<double> offsets, double offset) {
  var active = 0;
  for (var i = 0; i < offsets.length; i++) {
    // Half a header of slack so the highlight flips as a header reaches the
    // top rather than a pixel before.
    if (offsets[i] <= offset + _sectionHeaderExtent / 2) active = i;
  }
  return active;
}

/// One tappable standard emoji.
///
/// [keyPrefix] exists because the frequently-used section repeats emoji that
/// also appear in their own category, and both are in the same scroll view — two
/// widgets keyed `emoji-tile-fire` would be indistinguishable to a test.
class _EmojiTile extends StatelessWidget {
  final EmojiEntry entry;
  final VoidCallback onTap;
  final String keyPrefix;

  const _EmojiTile({
    required this.entry,
    required this.onTap,
    this.keyPrefix = 'emoji-tile',
  });

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      key: ValueKey('$keyPrefix-${entry.tileId}'),
      onTap: onTap,
      behavior: HitTestBehavior.opaque,
      child: Semantics(
        button: true,
        label: entry.name,
        child: Center(
          child: NativeEmojiGlyph(emoji: entry.native, size: _emojiGlyphSize),
        ),
      ),
    );
  }
}

/// One tappable custom (image) emoji. Same cell and same glyph size as
/// [_EmojiTile] — a community's emoji shouldn't be laid out on its own looser
/// grid, which is what made the custom tab read as a different component.
class _CustomEmojiTile extends StatelessWidget {
  final CustomEmoji emoji;
  final VoidCallback onTap;
  final String keyPrefix;

  const _CustomEmojiTile({
    required this.emoji,
    required this.onTap,
    this.keyPrefix = 'emoji-tile-custom',
  });

  @override
  Widget build(BuildContext context) {
    return GestureDetector(
      key: ValueKey('$keyPrefix-${emoji.shortcode}'),
      onTap: onTap,
      behavior: HitTestBehavior.opaque,
      child: Tooltip(
        message: ':${emoji.shortcode}:',
        child: Center(
          child: CustomEmojiImage(
            shortcode: emoji.shortcode,
            url: emoji.url,
            size: _emojiGlyphSize,
          ),
        ),
      ),
    );
  }
}

/// Every section in one scroll view, headers pinning as they reach the top.
///
/// Replaces the old tab-per-category swap: the rail is a shortcut into a single
/// list, not a set of separate pages, which is how both desktop and every
/// system emoji keyboard behave.
class _ContinuousEmojiGrid extends StatelessWidget {
  final List<_EmojiSection> sections;
  final ScrollController controller;

  const _ContinuousEmojiGrid({
    required this.sections,
    required this.controller,
  });

  @override
  Widget build(BuildContext context) {
    return CustomScrollView(
      key: const ValueKey('emoji-picker-grid'),
      controller: controller,
      slivers: [
        for (final section in sections)
          SliverMainAxisGroup(
            key: ValueKey('emoji-section-${section.id}'),
            slivers: [
              SliverPersistentHeader(
                pinned: true,
                delegate: _SectionHeaderDelegate(section.label),
              ),
              SliverPadding(
                padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
                sliver: SliverGrid.builder(
                  gridDelegate: _gridDelegate,
                  itemCount: section.itemCount,
                  itemBuilder: section.itemBuilder,
                ),
              ),
            ],
          ),
        // Clear the home indicator so the last row isn't half-swallowed. Sits
        // outside the sections so it doesn't skew their offsets.
        SliverToBoxAdapter(
          child: SizedBox(
            height: MediaQuery.viewPaddingOf(context).bottom + Grid.xxs,
          ),
        ),
      ],
    );
  }
}

/// Search results: custom emoji first (a community's own emoji are the ones a
/// user is most likely hunting for by name), then the standard set.
class _EmojiSearchResults extends StatelessWidget {
  final List<EmojiEntry> entries;
  final List<CustomEmoji> customEmoji;
  final void Function(String emoji) onSelect;

  const _EmojiSearchResults({
    required this.entries,
    required this.customEmoji,
    required this.onSelect,
  });

  @override
  Widget build(BuildContext context) {
    if (entries.isEmpty && customEmoji.isEmpty) {
      return _EmojiEmptyState(
        icon: LucideIcons.searchX,
        message: 'No emoji found.',
      );
    }

    return CustomScrollView(
      key: const ValueKey('emoji-picker-search-results'),
      slivers: [
        if (customEmoji.isNotEmpty) ...[
          const _SectionHeader(label: 'Custom'),
          SliverPadding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
            sliver: SliverGrid.builder(
              gridDelegate: _gridDelegate,
              itemCount: customEmoji.length,
              itemBuilder: (context, index) {
                final entry = customEmoji[index];
                return _CustomEmojiTile(
                  emoji: entry,
                  onTap: () => onSelect(':${entry.shortcode}:'),
                );
              },
            ),
          ),
        ],
        if (entries.isNotEmpty) ...[
          if (customEmoji.isNotEmpty) const _SectionHeader(label: 'Emoji'),
          SliverPadding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
            sliver: SliverGrid.builder(
              gridDelegate: _gridDelegate,
              itemCount: entries.length,
              itemBuilder: (context, index) {
                final entry = entries[index];
                return _EmojiTile(
                  entry: entry,
                  onTap: () => onSelect(entry.native),
                );
              },
            ),
          ),
        ],
        SliverToBoxAdapter(
          child: SizedBox(
            height: MediaQuery.viewPaddingOf(context).bottom + Grid.xxs,
          ),
        ),
      ],
    );
  }
}

/// Pinned header for a section of the continuous grid. Opaque, so rows scroll
/// under it rather than showing through.
class _SectionHeaderDelegate extends SliverPersistentHeaderDelegate {
  final String label;

  const _SectionHeaderDelegate(this.label);

  @override
  double get minExtent => _sectionHeaderExtent;

  @override
  double get maxExtent => _sectionHeaderExtent;

  @override
  Widget build(
    BuildContext context,
    double shrinkOffset,
    bool overlapsContent,
  ) {
    return Container(
      color: context.colors.surfaceContainerHighest,
      alignment: Alignment.centerLeft,
      padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
      child: Text(label, style: _sectionLabelStyle(context)),
    );
  }

  @override
  bool shouldRebuild(_SectionHeaderDelegate oldDelegate) =>
      oldDelegate.label != label;
}

/// Non-pinned header, for the search results list.
class _SectionHeader extends StatelessWidget {
  final String label;

  const _SectionHeader({required this.label});

  @override
  Widget build(BuildContext context) {
    return SliverToBoxAdapter(
      child: Container(
        height: _sectionHeaderExtent,
        alignment: Alignment.centerLeft,
        padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
        child: Text(label, style: _sectionLabelStyle(context)),
      ),
    );
  }
}

TextStyle? _sectionLabelStyle(BuildContext context) =>
    context.textTheme.labelMedium?.copyWith(
      color: context.colors.onSurfaceVariant,
      fontWeight: FontWeight.w600,
    );

class _EmojiEmptyState extends StatelessWidget {
  final IconData icon;
  final String message;

  const _EmojiEmptyState({required this.icon, required this.message});

  @override
  Widget build(BuildContext context) {
    return Center(
      child: Column(
        mainAxisSize: MainAxisSize.min,
        children: [
          Icon(icon, size: Grid.md, color: context.colors.onSurfaceVariant),
          const SizedBox(height: Grid.xxs),
          Text(
            message,
            style: context.textTheme.bodySmall?.copyWith(
              color: context.colors.onSurfaceVariant,
            ),
          ),
        ],
      ),
    );
  }
}
