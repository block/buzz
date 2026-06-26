import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../shared/theme/theme.dart';
import '../custom_emoji/custom_emoji.dart';
import '../custom_emoji/custom_emoji_provider.dart';
import '../custom_emoji/custom_emoji_render.dart';

/// Opens the full emoji picker as a modal bottom sheet.
void showEmojiPicker({
  required BuildContext context,
  required void Function(String emoji) onSelect,
}) {
  showModalBottomSheet<void>(
    context: context,
    isScrollControlled: true,
    showDragHandle: true,
    backgroundColor: Theme.of(context).colorScheme.surfaceContainerHighest,
    builder: (sheetContext) => EmojiPickerSheet(
      onSelect: (emoji) {
        Navigator.of(sheetContext).pop();
        onSelect(emoji);
      },
    ),
  );
}

/// Emoji categories for the picker. System Unicode emoji — no packages needed.
const emojiCategories = <({String label, IconData icon, List<String> emoji})>[
  (
    label: 'Popular',
    icon: LucideIcons.clock,
    emoji: [
      '\u{1F44D}',
      '\u{2764}\u{FE0F}',
      '\u{1F602}',
      '\u{1F389}',
      '\u{1F440}',
      '\u{1F64F}',
      '\u{1F525}',
      '\u{2705}',
    ],
  ),
  (
    label: 'Smileys',
    icon: LucideIcons.smile,
    emoji: [
      '\u{1F600}',
      '\u{1F603}',
      '\u{1F604}',
      '\u{1F601}',
      '\u{1F605}',
      '\u{1F602}',
      '\u{1F923}',
      '\u{1F607}',
      '\u{1F60A}',
      '\u{1F60D}',
      '\u{1F618}',
      '\u{1F617}',
      '\u{1F61A}',
      '\u{1F619}',
      '\u{1F60B}',
      '\u{1F61B}',
      '\u{1F61D}',
      '\u{1F61C}',
      '\u{1F911}',
      '\u{1F917}',
      '\u{1F914}',
      '\u{1F910}',
      '\u{1F928}',
      '\u{1F610}',
      '\u{1F611}',
      '\u{1F636}',
      '\u{1F60F}',
      '\u{1F612}',
      '\u{1F644}',
      '\u{1F62C}',
      '\u{1F925}',
      '\u{1F60C}',
      '\u{1F614}',
      '\u{1F62A}',
      '\u{1F924}',
      '\u{1F634}',
      '\u{1F637}',
      '\u{1F912}',
      '\u{1F915}',
      '\u{1F922}',
      '\u{1F92E}',
      '\u{1F927}',
      '\u{1F975}',
      '\u{1F976}',
      '\u{1F974}',
      '\u{1F635}',
      '\u{1F92F}',
      '\u{1F920}',
      '\u{1F973}',
      '\u{1F978}',
    ],
  ),
  (
    label: 'Gestures',
    icon: LucideIcons.hand,
    emoji: [
      '\u{1F44D}',
      '\u{1F44E}',
      '\u{1F44A}',
      '\u{270A}',
      '\u{1F91B}',
      '\u{1F91C}',
      '\u{1F44F}',
      '\u{1F64C}',
      '\u{1F450}',
      '\u{1F64F}',
      '\u{1F91D}',
      '\u{270C}\u{FE0F}',
      '\u{1F91E}',
      '\u{1F91F}',
      '\u{1F918}',
      '\u{1F448}',
      '\u{1F449}',
      '\u{1F446}',
      '\u{1F447}',
      '\u{261D}\u{FE0F}',
      '\u{1F4AA}',
      '\u{1F44B}',
      '\u{1F590}\u{FE0F}',
    ],
  ),
  (
    label: 'Objects',
    icon: LucideIcons.lightbulb,
    emoji: [
      '\u{2764}\u{FE0F}',
      '\u{1F525}',
      '\u{2B50}',
      '\u{1F31F}',
      '\u{1F4A5}',
      '\u{1F389}',
      '\u{1F38A}',
      '\u{1F3C6}',
      '\u{1F947}',
      '\u{1F4A1}',
      '\u{1F4AF}',
      '\u{2705}',
      '\u{274C}',
      '\u{26A0}\u{FE0F}',
      '\u{1F6A8}',
      '\u{1F4DD}',
      '\u{1F4CB}',
      '\u{1F4CC}',
      '\u{1F517}',
      '\u{1F4E3}',
      '\u{1F514}',
      '\u{1F3B5}',
      '\u{1F3B6}',
      '\u{1F680}',
    ],
  ),
  (
    label: 'Nature',
    icon: LucideIcons.leaf,
    emoji: [
      '\u{1F331}',
      '\u{1F332}',
      '\u{1F333}',
      '\u{1F334}',
      '\u{1F335}',
      '\u{1F33B}',
      '\u{1F33A}',
      '\u{1F337}',
      '\u{1F339}',
      '\u{1F340}',
      '\u{1F341}',
      '\u{1F343}',
      '\u{1F31E}',
      '\u{1F308}',
      '\u{2600}\u{FE0F}',
      '\u{1F327}\u{FE0F}',
      '\u{26A1}',
      '\u{2744}\u{FE0F}',
      '\u{1F30A}',
      '\u{1F436}',
      '\u{1F431}',
      '\u{1F98A}',
      '\u{1F42C}',
      '\u{1F985}',
    ],
  ),
];

class EmojiPickerSheet extends ConsumerStatefulWidget {
  final void Function(String emoji) onSelect;

  const EmojiPickerSheet({super.key, required this.onSelect});

  @override
  ConsumerState<EmojiPickerSheet> createState() => _EmojiPickerSheetState();
}

class _EmojiPickerSheetState extends ConsumerState<EmojiPickerSheet> {
  /// -1 = "All", 0..N = specific category.
  int _selectedCategory = -1;

  static final _allEmoji = () {
    final seen = <String>{};
    return [
      for (final cat in emojiCategories)
        for (final e in cat.emoji)
          if (seen.add(e)) e,
    ];
  }();

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).colorScheme;
    final customEmoji = ref.watch(customEmojiListProvider);
    final isCustomCategory =
        customEmoji.isNotEmpty && _selectedCategory == emojiCategories.length;
    final emoji = _selectedCategory < 0
        ? _allEmoji
        : _selectedCategory < emojiCategories.length
        ? emojiCategories[_selectedCategory].emoji
        : const <String>[];

    return SizedBox(
      height: 340,
      child: Column(
        children: [
          // Category icon bar.
          SizedBox(
            height: 40,
            child: Row(
              children: [
                const SizedBox(width: Grid.twelve),
                CategoryIcon(
                  icon: LucideIcons.layoutGrid,
                  selected: _selectedCategory < 0,
                  onTap: () => setState(() => _selectedCategory = -1),
                ),
                for (var i = 0; i < emojiCategories.length; i++)
                  CategoryIcon(
                    icon: emojiCategories[i].icon,
                    selected: _selectedCategory == i,
                    onTap: () => setState(() => _selectedCategory = i),
                  ),
                if (customEmoji.isNotEmpty)
                  CategoryIcon(
                    icon: LucideIcons.sparkles,
                    selected: isCustomCategory,
                    onTap: () => setState(
                      () => _selectedCategory = emojiCategories.length,
                    ),
                  ),
              ],
            ),
          ),
          Divider(height: 1, color: colors.outlineVariant),
          const SizedBox(height: Grid.xxs),
          Expanded(
            child: isCustomCategory
                ? _CustomEmojiGrid(
                    emoji: customEmoji,
                    onSelect: widget.onSelect,
                  )
                : GridView.builder(
                    padding: const EdgeInsets.symmetric(horizontal: Grid.xs),
                    gridDelegate:
                        const SliverGridDelegateWithFixedCrossAxisCount(
                          crossAxisCount: 8,
                          mainAxisSpacing: Grid.half,
                          crossAxisSpacing: Grid.half,
                        ),
                    itemCount: emoji.length,
                    itemBuilder: (context, index) {
                      final e = emoji[index];
                      return GestureDetector(
                        onTap: () => widget.onSelect(e),
                        child: Center(
                          child: Text(e, style: const TextStyle(fontSize: 28)),
                        ),
                      );
                    },
                  ),
          ),
        ],
      ),
    );
  }
}

class _CustomEmojiGrid extends StatelessWidget {
  final List<CustomEmoji> emoji;
  final void Function(String emoji) onSelect;

  const _CustomEmojiGrid({required this.emoji, required this.onSelect});

  @override
  Widget build(BuildContext context) {
    return GridView.builder(
      padding: const EdgeInsets.symmetric(horizontal: Grid.xs),
      gridDelegate: const SliverGridDelegateWithFixedCrossAxisCount(
        crossAxisCount: 6,
        mainAxisSpacing: Grid.half,
        crossAxisSpacing: Grid.half,
      ),
      itemCount: emoji.length,
      itemBuilder: (context, index) {
        final entry = emoji[index];
        return GestureDetector(
          onTap: () => onSelect(':${entry.shortcode}:'),
          child: Tooltip(
            message: ':${entry.shortcode}:',
            child: Center(
              child: CustomEmojiImage(
                shortcode: entry.shortcode,
                url: entry.url,
                size: 32,
              ),
            ),
          ),
        );
      },
    );
  }
}

class CategoryIcon extends StatelessWidget {
  final IconData icon;
  final bool selected;
  final VoidCallback onTap;

  const CategoryIcon({
    super.key,
    required this.icon,
    required this.selected,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).colorScheme;
    return SizedBox(
      width: 40,
      height: 40,
      child: IconButton(
        onPressed: onTap,
        icon: Icon(
          icon,
          size: 18,
          color: selected ? colors.primary : colors.onSurfaceVariant,
        ),
        padding: EdgeInsets.zero,
        visualDensity: VisualDensity.compact,
      ),
    );
  }
}
