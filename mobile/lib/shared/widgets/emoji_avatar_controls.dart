import 'dart:async';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../emoji/emoji_data.dart';
import '../emoji/emoji_search.dart';
import '../theme/theme.dart';
import '../theme/buzz_icons.dart';
import 'avatar_background_grid.dart';
import 'avatar_editor_option_button.dart';
import 'emoji_avatar_tile.dart';
import 'ios_glass_navigation_button.dart';
import 'ios_native_skin_tone_control.dart';

const _skinTones = [
  (label: 'Default', color: Color(0xFFFFC93A)),
  (label: 'Light', color: Color(0xFFFFDAB7)),
  (label: 'Medium-light', color: Color(0xFFE7B98F)),
  (label: 'Medium', color: Color(0xFFC88C61)),
  (label: 'Medium-dark', color: Color(0xFFA46134)),
  (label: 'Dark', color: Color(0xFF5D4437)),
];

/// The two editing trays shared by profile and agent avatars.
enum EmojiEditorSection { background, emoji }

/// Shared search, skin-tone, six-column emoji grid, and background controls.
class EmojiAvatarControls extends HookConsumerWidget {
  const EmojiAvatarControls({
    super.key,
    required this.height,
    required this.activeSection,
    required this.dataset,
    required this.selectedEmoji,
    required this.selectedColor,
    required this.transitionProgress,
    required this.transitionDirection,
    this.controlsFromBelow = false,
    required this.onSectionChanged,
    required this.onEmojiSelected,
    required this.onColorSelected,
  });

  final double height;
  final EmojiEditorSection activeSection;
  final EmojiDataset dataset;
  final String selectedEmoji;
  final int selectedColor;
  final double transitionProgress;
  final double transitionDirection;
  final bool controlsFromBelow;
  final ValueChanged<EmojiEditorSection> onSectionChanged;
  final ValueChanged<String> onEmojiSelected;
  final ValueChanged<int> onColorSelected;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final trayOffset = reduceMotion ? 0.0 : 16 * (1 - transitionProgress);
    final actionOffset = reduceMotion
        ? 0.0
        : transitionDirection * 24.0 * (1 - transitionProgress);
    final skinTone = useState(_skinToneForEmoji(dataset, selectedEmoji));
    final seededSkinTone = useRef(false);
    useEffect(() {
      if (seededSkinTone.value || dataset.isEmpty) return null;
      seededSkinTone.value = true;
      final initialTone = _skinToneForEmoji(dataset, selectedEmoji);
      if (skinTone.value != initialTone) skinTone.value = initialTone;
      return null;
    }, [dataset]);
    final searchController = useTextEditingController();
    useListenable(searchController);
    final visibleEmoji = useMemoized(() {
      final entries = _emojiForSkinTone(dataset, skinTone.value);
      final query = searchController.text.trim();
      return query.isEmpty ? entries : searchEmoji(query, entries);
    }, [dataset, skinTone.value, searchController.text]);

    void selectSkinTone(int next) {
      if (skinTone.value == next) return;
      unawaited(HapticFeedback.selectionClick());
      skinTone.value = next;
    }

    return SizedBox(
      key: const ValueKey('emoji-avatar-picker-content'),
      height: height,
      child: Column(
        children: [
          Expanded(
            child: ClipRect(
              child: Transform.translate(
                key: const ValueKey('avatar-mode-tray-transition-transform'),
                offset: Offset(0, trayOffset),
                child: Opacity(
                  opacity: transitionProgress,
                  child: activeSection == EmojiEditorSection.background
                      ? Center(
                          key: const ValueKey(
                            'emoji-background-editor-alignment',
                          ),
                          child: AvatarBackgroundGrid(
                            key: const ValueKey('emoji-background-editor'),
                            selectedColor: selectedColor,
                            onColorSelected: onColorSelected,
                            colorKeyPrefix: 'emoji-avatar-color',
                          ),
                        )
                      : Column(
                          key: const ValueKey('emoji-glyph-editor'),
                          children: [
                            Row(
                              children: [
                                Expanded(
                                  child: TextField(
                                    key: const ValueKey('emoji-avatar-search'),
                                    controller: searchController,
                                    textInputAction: TextInputAction.search,
                                    decoration: InputDecoration(
                                      hintText: 'Search emoji',
                                      filled: true,
                                      fillColor: context
                                          .colors
                                          .surfaceContainerHighest,
                                      contentPadding:
                                          const EdgeInsets.symmetric(
                                            vertical: 10,
                                          ),
                                      border: const OutlineInputBorder(
                                        borderRadius: BorderRadius.all(
                                          Radius.circular(Radii.full),
                                        ),
                                        borderSide: BorderSide.none,
                                      ),
                                      enabledBorder: const OutlineInputBorder(
                                        borderRadius: BorderRadius.all(
                                          Radius.circular(Radii.full),
                                        ),
                                        borderSide: BorderSide.none,
                                      ),
                                      focusedBorder: const OutlineInputBorder(
                                        borderRadius: BorderRadius.all(
                                          Radius.circular(Radii.full),
                                        ),
                                        borderSide: BorderSide.none,
                                      ),
                                      prefixIcon: const Icon(
                                        BuzzIcons.search,
                                        size: 20,
                                      ),
                                      suffixIcon: searchController.text.isEmpty
                                          ? null
                                          : IconButton(
                                              tooltip: 'Clear search',
                                              onPressed: searchController.clear,
                                              icon: const Icon(
                                                BuzzIcons.x,
                                                size: 18,
                                              ),
                                            ),
                                    ),
                                  ),
                                ),
                                const SizedBox(width: Grid.xxs),
                                _AvatarSkinToneSelector(
                                  value: skinTone.value,
                                  onChanged: selectSkinTone,
                                ),
                              ],
                            ),
                            const SizedBox(height: Grid.xxs),
                            Expanded(
                              child: dataset.isEmpty
                                  ? const Center(
                                      child: CircularProgressIndicator(),
                                    )
                                  : visibleEmoji.isEmpty
                                  ? Center(
                                      child: Text(
                                        'No emoji found',
                                        style: context.textTheme.bodyMedium
                                            ?.copyWith(
                                              color: context
                                                  .colors
                                                  .onSurfaceVariant,
                                            ),
                                      ),
                                    )
                                  : GridView.builder(
                                      key: const ValueKey('emoji-avatar-grid'),
                                      padding: EdgeInsets.zero,
                                      gridDelegate:
                                          const SliverGridDelegateWithFixedCrossAxisCount(
                                            crossAxisCount: 6,
                                          ),
                                      itemCount: visibleEmoji.length,
                                      itemBuilder: (context, index) {
                                        final entry = visibleEmoji[index];
                                        final isSelected =
                                            entry.native == selectedEmoji;
                                        void selectEmoji() {
                                          unawaited(
                                            HapticFeedback.selectionClick(),
                                          );
                                          onEmojiSelected(entry.native);
                                        }

                                        return EmojiAvatarTile(
                                          emoji: entry.native,
                                          label: entry.name,
                                          tileId: entry.tileId,
                                          isSelected: isSelected,
                                          onTap: selectEmoji,
                                        );
                                      },
                                    ),
                            ),
                          ],
                        ),
                ),
              ),
            ),
          ),
          const SizedBox(height: Grid.xs),
          ClipRect(
            child: Transform.translate(
              key: const ValueKey('avatar-mode-transition-transform'),
              offset: controlsFromBelow
                  ? Offset(0, trayOffset)
                  : Offset(actionOffset, 0),
              child: Opacity(
                opacity: transitionProgress,
                child: Row(
                  crossAxisAlignment: CrossAxisAlignment.start,
                  children: [
                    const Spacer(),
                    Expanded(
                      child: AvatarEditorOptionButton(
                        key: const ValueKey('emoji-editor-background'),
                        icon: BuzzIcons.palette,
                        iosIcon: IosGlassNavigationIcon.palette,
                        label: 'Background',
                        selected:
                            activeSection == EmojiEditorSection.background,
                        onTap: () =>
                            onSectionChanged(EmojiEditorSection.background),
                        labelMaxWidth: 96,
                      ),
                    ),
                    const SizedBox(width: Grid.half),
                    Expanded(
                      child: AvatarEditorOptionButton(
                        key: const ValueKey('emoji-editor-emoji'),
                        icon: BuzzIcons.smile,
                        iosIcon: IosGlassNavigationIcon.emoji,
                        label: 'Emoji',
                        selected: activeSection == EmojiEditorSection.emoji,
                        onTap: () => onSectionChanged(EmojiEditorSection.emoji),
                        labelMaxWidth: 80,
                      ),
                    ),
                    const Spacer(),
                  ],
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

class _AvatarSkinToneSelector extends StatelessWidget {
  const _AvatarSkinToneSelector({required this.value, required this.onChanged});

  final int value;
  final ValueChanged<int> onChanged;

  @override
  Widget build(BuildContext context) {
    final selected = _skinTones[_validSkinTone(value)];
    if (defaultTargetPlatform == TargetPlatform.iOS) {
      return IosNativeSkinToneControl(
        key: const ValueKey('emoji-avatar-skin-tone'),
        value: value,
        colors: [for (final tone in _skinTones) tone.color],
        labels: [for (final tone in _skinTones) tone.label],
        onChanged: onChanged,
      );
    }
    return PopupMenuButton<int>(
      key: const ValueKey('emoji-avatar-skin-tone'),
      initialValue: value,
      tooltip: 'Skin tone',
      position: PopupMenuPosition.under,
      onSelected: onChanged,
      itemBuilder: (context) => [
        for (final (index, tone) in _skinTones.indexed)
          PopupMenuItem<int>(
            value: index,
            child: Row(
              children: [
                _SkinToneDot(color: tone.color),
                const SizedBox(width: Grid.xs),
                Expanded(child: Text(tone.label)),
                if (index == value)
                  Icon(
                    BuzzIcons.check,
                    size: 18,
                    color: context.colors.primary,
                  ),
              ],
            ),
          ),
      ],
      child: SizedBox.square(
        dimension: 48,
        child: Center(child: _SkinToneDot(color: selected.color)),
      ),
    );
  }
}

class _SkinToneDot extends StatelessWidget {
  const _SkinToneDot({required this.color});

  final Color color;

  @override
  Widget build(BuildContext context) => Container(
    width: 22,
    height: 22,
    decoration: BoxDecoration(
      color: color,
      shape: BoxShape.circle,
      border: Border.all(color: context.colors.outlineVariant),
    ),
  );
}

int _validSkinTone(int? value) =>
    value != null && value >= 0 && value < _skinTones.length ? value : 0;

int _skinToneForEmoji(EmojiDataset dataset, String emoji) =>
    dataset.all
        .where((entry) => entry.native == emoji)
        .map((entry) => _validSkinTone(entry.skinIndex))
        .firstOrNull ??
    0;

List<EmojiEntry> _emojiForSkinTone(EmojiDataset dataset, int skinTone) {
  final variantsById = <String, List<EmojiEntry>>{};
  for (final entry in dataset.all) {
    variantsById.putIfAbsent(entry.id, () => []).add(entry);
  }
  return [
    for (final variants in variantsById.values)
      variants.firstWhere(
        (entry) => entry.skinIndex == skinTone,
        orElse: () => variants.firstWhere(
          (entry) => entry.skinIndex == 0,
          orElse: () => variants.first,
        ),
      ),
  ];
}
