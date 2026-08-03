import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/custom_emoji/custom_emoji.dart';
import '../../shared/custom_emoji/custom_emoji_provider.dart';
import '../../shared/custom_emoji/custom_emoji_render.dart';
import '../../shared/emoji/emoji_data.dart';
import '../../shared/emoji/emoji_data_provider.dart';
import '../../shared/emoji/emoji_search.dart';
import '../../shared/emoji/native_emoji_glyph.dart';
import '../../shared/theme/theme.dart';
import 'recent_emoji_provider.dart';

part 'emoji_picker/search_field.dart';
part 'emoji_picker/category_rail.dart';
part 'emoji_picker/emoji_grid.dart';

/// Height of the picker sheet as a fraction of the screen. The full emoji set
/// is ~1.9k glyphs; the old fixed 340px sheet only ever showed a hand-picked
/// subset and had no room to browse.
const _sheetHeightFactor = 0.62;

/// Opens the full emoji picker as a modal bottom sheet.
///
/// [onSelect] receives a single string, normalized the same way desktop's
/// `EmojiPicker` normalizes its selection: a standard emoji emits its glyph, a
/// custom emoji emits `:shortcode:`. Callers store or send that string and let
/// the existing renderers resolve it.
void showEmojiPicker({
  required BuildContext context,
  required void Function(String emoji) onSelect,
  VoidCallback? onDismiss,
}) {
  showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    backgroundColor: context.colors.surfaceContainerHighest,
    builder: (sheetContext) => EmojiPickerSheet(
      onSelect: (emoji) {
        Navigator.of(sheetContext).pop();
        onSelect(emoji);
      },
    ),
  ).whenComplete(onDismiss ?? () {});
}

class EmojiPickerSheet extends HookConsumerWidget {
  final void Function(String emoji) onSelect;

  const EmojiPickerSheet({super.key, required this.onSelect});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final dataset = ref.watch(emojiDatasetOrEmptyProvider);
    final customEmoji = ref.watch(customEmojiListProvider);
    final recent = ref.watch(recentEmojiProvider);

    final searchController = useTextEditingController();
    final query = useState('');
    useEffect(() {
      void onChanged() => query.value = searchController.text;
      searchController.addListener(onChanged);
      return () => searchController.removeListener(onChanged);
    }, [searchController]);

    final trimmedQuery = query.value.trim();
    final isSearching = trimmedQuery.isNotEmpty;

    void select(String emoji) => onSelect(emoji);

    final sections = useMemoized(
      () => _buildSections(
        dataset: dataset,
        customEmoji: customEmoji,
        recent: recent,
        onSelect: select,
      ),
      [dataset, customEmoji, recent],
    );
    final offsets = useMemoized(() => _sectionOffsets(sections), [sections]);

    final scrollController = useScrollController();
    // A notifier rather than state: the highlight changes on every scroll frame
    // and only the rail needs to hear about it. Rebuilding the sheet would
    // rebuild the grid underneath it.
    final activeSection = useMemoized(() => ValueNotifier(0), [sections]);
    useEffect(() => activeSection.dispose, [activeSection]);

    useEffect(() {
      void onScroll() {
        if (!scrollController.hasClients) return;
        activeSection.value = _activeSectionIndex(
          offsets,
          scrollController.offset,
        );
      }

      scrollController.addListener(onScroll);
      return () => scrollController.removeListener(onScroll);
    }, [scrollController, offsets, activeSection]);

    void jumpToSection(int index) {
      activeSection.value = index;
      if (!scrollController.hasClients) return;
      final max = scrollController.position.maxScrollExtent;
      scrollController.animateTo(
        offsets[index].clamp(0.0, max),
        duration: const Duration(milliseconds: 240),
        curve: Curves.easeOutCubic,
      );
    }

    // Recompute only when the query or the underlying sets change — scanning
    // ~1.9k entries per keystroke is sub-millisecond, but rebuilds are frequent
    // while the sheet animates.
    final results = useMemoized(
      () => isSearching
          ? searchEmoji(trimmedQuery, dataset.all)
          : const <EmojiEntry>[],
      [trimmedQuery, dataset],
    );
    final customResults = useMemoized(
      () => isSearching
          ? rankByShortcode(
              trimmedQuery,
              customEmoji,
              (emoji) => emoji.shortcode,
            )
          : const <CustomEmoji>[],
      [trimmedQuery, customEmoji],
    );

    return SizedBox(
      height: MediaQuery.sizeOf(context).height * _sheetHeightFactor,
      child: Column(
        children: [
          _EmojiSearchField(controller: searchController),
          if (!isSearching && sections.isNotEmpty)
            ValueListenableBuilder<int>(
              valueListenable: activeSection,
              builder: (context, active, _) => _CategoryRail(
                sections: sections,
                activeIndex: active,
                onSelect: jumpToSection,
              ),
            ),
          Divider(height: 1, color: context.colors.outlineVariant),
          Expanded(
            child: dataset.isEmpty && customEmoji.isEmpty
                ? const Center(child: CircularProgressIndicator())
                : isSearching
                ? _EmojiSearchResults(
                    entries: results,
                    customEmoji: customResults,
                    onSelect: select,
                  )
                : _ContinuousEmojiGrid(
                    sections: sections,
                    controller: scrollController,
                  ),
          ),
        ],
      ),
    );
  }
}

/// Build the scroll order: frequently-used, the dataset's own categories in
/// emoji-mart order, then the community's custom emoji.
///
/// Frequently-used is omitted rather than shown empty — in a continuous list an
/// empty section is a gap with a label on it, and the rail entry would lead
/// nowhere.
List<_EmojiSection> _buildSections({
  required EmojiDataset dataset,
  required List<CustomEmoji> customEmoji,
  required List<RecentEmojiEntry> recent,
  required void Function(String emoji) onSelect,
}) {
  final sections = <_EmojiSection>[];

  final recentTiles = _resolveRecentTiles(
    recent: recent,
    dataset: dataset,
    customEmoji: customEmoji,
    onSelect: onSelect,
  );
  if (recentTiles.isNotEmpty) {
    sections.add(
      _EmojiSection(
        id: 'frequent',
        label: 'Frequently used',
        icon: LucideIcons.clock,
        itemCount: recentTiles.length,
        itemBuilder: (context, index) => recentTiles[index],
      ),
    );
  }

  for (final category in dataset.categories) {
    sections.add(
      _EmojiSection(
        id: category.id,
        label: category.label,
        icon: _categoryIcon(category.id),
        itemCount: category.emoji.length,
        itemBuilder: (context, index) {
          final entry = category.emoji[index];
          return _EmojiTile(entry: entry, onTap: () => onSelect(entry.native));
        },
      ),
    );
  }

  if (customEmoji.isNotEmpty) {
    sections.add(
      _EmojiSection(
        id: 'custom',
        label: 'Custom',
        icon: LucideIcons.sparkles,
        itemCount: customEmoji.length,
        itemBuilder: (context, index) {
          final entry = customEmoji[index];
          return _CustomEmojiTile(
            emoji: entry,
            onTap: () => onSelect(':${entry.shortcode}:'),
          );
        },
      ),
    );
  }

  return sections;
}

/// Resolve the recency list back to renderable tiles.
///
/// Entries are stored as the selected string, so a standard emoji is a glyph and
/// a custom one is `:shortcode:` — resolve each against the dataset and the
/// palette, and drop anything that no longer exists (a custom emoji removed from
/// the community would otherwise render as literal text).
List<Widget> _resolveRecentTiles({
  required List<RecentEmojiEntry> recent,
  required EmojiDataset dataset,
  required List<CustomEmoji> customEmoji,
  required void Function(String emoji) onSelect,
}) {
  final customByShortcode = {
    for (final emoji in customEmoji) emoji.shortcode.toLowerCase(): emoji,
  };
  final entriesByNative = {
    for (final entry in dataset.all) entry.native: entry,
  };

  final tiles = <Widget>[];
  for (final item in recent) {
    final value = item.emoji;
    if (value.startsWith(':') && value.endsWith(':')) {
      final custom =
          customByShortcode[value.substring(1, value.length - 1).toLowerCase()];
      if (custom == null) continue;
      tiles.add(
        _CustomEmojiTile(
          emoji: custom,
          onTap: () => onSelect(value),
          keyPrefix: 'emoji-tile-frequent-custom',
        ),
      );
      continue;
    }
    final entry = entriesByNative[value];
    if (entry == null) continue;
    tiles.add(
      _EmojiTile(
        entry: entry,
        onTap: () => onSelect(entry.native),
        keyPrefix: 'emoji-tile-frequent',
      ),
    );
  }
  return tiles;
}
