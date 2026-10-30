part of '../emoji_picker.dart';

/// Catalog results the local palette does not already show.
List<CustomEmoji> _catalogResultsToShow(
  List<CustomEmoji> catalog,
  List<CustomEmoji> local,
) {
  final localByShortcode = {for (final emoji in local) emoji.shortcode: emoji};
  return [
    for (final emoji in catalog)
      if (localByShortcode[emoji.shortcode] != emoji) emoji,
  ];
}

/// The relay catalog part of the search results: a grid, a busy marker while
/// a search runs, or an error with Retry. Local results stay above it in every
/// case.
class _CatalogResultsSliver extends HookConsumerWidget {
  const _CatalogResultsSliver({
    required this.state,
    required this.results,
    required this.onSelect,
  });

  final EmojiCatalogSearchState state;
  final List<CustomEmoji> results;
  final void Function(CustomEmoji emoji) onSelect;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final colors = context.colors;
    return SliverMainAxisGroup(
      key: const ValueKey('emoji-catalog-results'),
      slivers: [
        if (results.isNotEmpty) ...[
          const _SectionHeader(label: 'Community catalog'),
          SliverPadding(
            padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
            sliver: SliverGrid.builder(
              gridDelegate: _gridDelegate,
              itemCount: results.length,
              itemBuilder: (context, index) {
                final emoji = results[index];
                return _CustomEmojiTile(
                  emoji: emoji,
                  onTap: () => onSelect(emoji),
                  keyPrefix: 'emoji-tile-catalog',
                );
              },
            ),
          ),
        ],
        if (state.failed)
          SliverToBoxAdapter(
            child: Padding(
              key: const ValueKey('emoji-catalog-error'),
              padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
              child: Row(
                children: [
                  Expanded(
                    child: Semantics(
                      liveRegion: true,
                      child: Text(
                        'Couldn’t search the emoji catalog.',
                        style: context.textTheme.bodySmall?.copyWith(
                          color: colors.onSurfaceVariant,
                        ),
                      ),
                    ),
                  ),
                  TextButton(
                    key: const ValueKey('emoji-catalog-retry'),
                    onPressed: () => unawaited(
                      ref.read(emojiCatalogSearchProvider.notifier).retry(),
                    ),
                    child: const Text('Retry'),
                  ),
                ],
              ),
            ),
          )
        else if (state.loading)
          SliverToBoxAdapter(
            child: Padding(
              padding: const EdgeInsets.all(Grid.xs),
              child: Center(
                child: Semantics(
                  label: 'Searching the emoji catalog',
                  liveRegion: true,
                  child: const SizedBox(
                    key: ValueKey('emoji-catalog-loading'),
                    width: Grid.sm,
                    height: Grid.sm,
                    child: CircularProgressIndicator(strokeWidth: 2),
                  ),
                ),
              ),
            ),
          ),
      ],
    );
  }
}
