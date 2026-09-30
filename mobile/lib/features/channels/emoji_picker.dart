import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
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
import '../../shared/relay/relay.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/buzz_sheet_header.dart';
import '../../shared/widgets/modal_presentation.dart';
import 'recent_emoji_provider.dart';

part 'emoji_picker/search_field.dart';
part 'emoji_picker/category_rail.dart';
part 'emoji_picker/emoji_grid.dart';
part 'emoji_picker/ios_native_picker.dart';

/// Android keeps the established Flutter tray height. iOS is presented by a
/// native sheet with system detents in [ios_native_picker.dart].
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
  if (defaultTargetPlatform == TargetPlatform.iOS) {
    unawaited(
      _presentIosEmojiPicker(
        context: context,
        onSelect: onSelect,
        onDismiss: onDismiss,
      ),
    );
    return;
  }

  _showFlutterEmojiPicker(
    context: context,
    onSelect: onSelect,
    onDismiss: onDismiss,
  );
}

void _showFlutterEmojiPicker({
  required BuildContext context,
  required void Function(String emoji) onSelect,
  VoidCallback? onDismiss,
}) {
  showBuzzModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showCloseButton: false,
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
    return SizedBox(
      height: MediaQuery.sizeOf(context).height * _sheetHeightFactor,
      child: _EmojiPickerContent(onSelect: onSelect),
    );
  }
}

class _EmojiPickerContent extends HookConsumerWidget {
  const _EmojiPickerContent({required this.onSelect});

  final void Function(String emoji) onSelect;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final dataset = ref.watch(emojiDatasetOrEmptyProvider);
    final customEmoji = ref.watch(customEmojiListProvider);
    final recent = ref.watch(recentEmojiProvider);
    final prefs = ref.read(savedPrefsProvider);
    final skinTone = useState(
      _validSkinTone(prefs.getInt(_emojiSkinTonePrefsKey)),
    );

    void selectSkinTone(int value) {
      final next = _validSkinTone(value);
      if (skinTone.value == next) return;
      skinTone.value = next;
      unawaited(prefs.setInt(_emojiSkinTonePrefsKey, next));
    }

    final visibleDataset = useMemoized(
      () => _datasetForSkinTone(dataset, skinTone.value),
      [dataset, skinTone.value],
    );

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
        dataset: visibleDataset,
        sourceDataset: dataset,
        customEmoji: customEmoji,
        recent: recent,
        onSelect: select,
      ),
      [visibleDataset, dataset, customEmoji, recent],
    );
    final offsets = useMemoized(() => _sectionOffsets(sections), [sections]);
    final scrollController = useScrollController();

    // A notifier rather than state: the highlight changes on every scroll frame
    // and only the rail needs to hear about it. Rebuilding the sheet would
    // rebuild the grid underneath it.
    //
    // Seed it from the current scroll offset rather than 0: a skin-tone change
    // rebuilds [sections] and so replaces this notifier, but the grid keeps its
    // scroll position (same controller, same section extents). Resetting to 0
    // here would falsely highlight the first category until the next scroll.
    final activeSection = useMemoized(
      () => ValueNotifier(
        scrollController.hasClients
            ? _activeSectionIndex(
                offsets,
                scrollController.offset,
                maxScrollExtent: scrollController.position.maxScrollExtent,
              )
            : 0,
      ),
      [sections],
    );
    useEffect(() => activeSection.dispose, [activeSection]);

    useEffect(() {
      void onScroll() {
        if (!scrollController.hasClients) return;
        activeSection.value = _activeSectionIndex(
          offsets,
          scrollController.offset,
          maxScrollExtent: scrollController.position.maxScrollExtent,
        );
      }

      scrollController.addListener(onScroll);
      return () => scrollController.removeListener(onScroll);
    }, [scrollController, offsets, activeSection]);

    void jumpToSection(int index) {
      activeSection.value = index;
      if (!scrollController.hasClients) return;
      final max = scrollController.position.maxScrollExtent;
      // The rail is a frequent navigation shortcut. An instant jump cannot
      // remain active and pull against a drag that begins immediately after it.
      scrollController.jumpTo(offsets[index].clamp(0.0, max));
    }

    // Recompute only when the query or the underlying sets change — scanning
    // ~1.9k entries per keystroke is sub-millisecond, but rebuilds are frequent
    // while the sheet animates.
    final results = useMemoized(
      () => isSearching
          ? searchEmoji(trimmedQuery, visibleDataset.all)
          : const <EmojiEntry>[],
      [trimmedQuery, visibleDataset],
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

    return Column(
      children: [
        LayoutBuilder(
          builder: (context, constraints) => BuzzSheetHeader(
            showDragHandle: true,
            leading: SizedBox(
              width: constraints.maxWidth - Grid.gutter * 2 - 44 - Grid.xxs,
              child: _EmojiSearchField(
                controller: searchController,
                padding: EdgeInsets.zero,
              ),
            ),
          ),
        ),
        if (!isSearching && sections.isNotEmpty)
          ValueListenableBuilder<int>(
            valueListenable: activeSection,
            builder: (context, active, _) => _CategoryRail(
              sections: sections,
              activeIndex: active,
              onSelect: jumpToSection,
              skinTone: skinTone.value,
              onSkinToneChanged: selectSkinTone,
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
                  controller: scrollController,
                )
              : _ContinuousEmojiGrid(
                  sections: sections,
                  controller: scrollController,
                ),
        ),
      ],
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
  required EmojiDataset sourceDataset,
  required List<CustomEmoji> customEmoji,
  required List<RecentEmojiEntry> recent,
  required void Function(String emoji) onSelect,
}) {
  final sections = <_EmojiSection>[];

  final recentTiles = _resolveRecentTiles(
    recent: recent,
    dataset: dataset,
    sourceDataset: sourceDataset,
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
  required EmojiDataset sourceDataset,
  required List<CustomEmoji> customEmoji,
  required void Function(String emoji) onSelect,
}) {
  final customByShortcode = {
    for (final emoji in customEmoji) emoji.shortcode.toLowerCase(): emoji,
  };
  final sourceEntriesByNative = {
    for (final entry in sourceDataset.all) entry.native: entry,
  };
  final visibleEntriesById = {for (final entry in dataset.all) entry.id: entry};
  final seenStandardIds = <String>{};

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
    final sourceEntry = sourceEntriesByNative[value];
    if (sourceEntry == null || !seenStandardIds.add(sourceEntry.id)) continue;
    final entry = visibleEntriesById[sourceEntry.id];
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

/// Project the dataset to one visible tile per shortcode. Emoji that support
/// skin tones use the selected variant; everything else keeps its default.
EmojiDataset _datasetForSkinTone(EmojiDataset dataset, int skinTone) {
  if (dataset.isEmpty) return dataset;
  final categories = <EmojiCategory>[];
  final all = <EmojiEntry>[];

  for (final category in dataset.categories) {
    final variantsById = <String, List<EmojiEntry>>{};
    for (final entry in category.emoji) {
      variantsById.putIfAbsent(entry.id, () => []).add(entry);
    }
    final visible = <EmojiEntry>[];
    for (final variants in variantsById.values) {
      final selected = variants.firstWhere(
        (entry) => entry.skinIndex == skinTone,
        orElse: () => variants.firstWhere(
          (entry) => entry.skinIndex == 0,
          orElse: () => variants.first,
        ),
      );
      visible.add(selected);
      all.add(selected);
    }
    categories.add(EmojiCategory(id: category.id, emoji: visible));
  }

  return EmojiDataset(
    categories: categories,
    all: all,
    nativeToShortcode: dataset.nativeToShortcode,
  );
}
