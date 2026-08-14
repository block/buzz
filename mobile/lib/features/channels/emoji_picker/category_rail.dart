part of '../emoji_picker.dart';

/// Rail icon for each emoji-mart category. Ids come from the dataset, so this
/// map is keyed on emoji-mart's own category ids.
IconData _categoryIcon(String categoryId) => switch (categoryId) {
  'people' => LucideIcons.smile,
  'nature' => LucideIcons.leaf,
  'foods' => LucideIcons.coffee,
  'activity' => LucideIcons.volleyball,
  'places' => LucideIcons.plane,
  'objects' => LucideIcons.lightbulb,
  'symbols' => LucideIcons.heart,
  'flags' => LucideIcons.flag,
  _ => LucideIcons.layoutGrid,
};

/// Height of the rail. Sized to a comfortable tap target rather than to the
/// 18px icon it holds.
const _railHeight = 36.0;

const _emojiSkinTonePrefsKey = 'buzz.emoji-picker.skin-tone.v1';

const _skinTones = [
  (label: 'Default', color: Color(0xFFFFC93A)),
  (label: 'Light', color: Color(0xFFFFDAB7)),
  (label: 'Medium-light', color: Color(0xFFE7B98F)),
  (label: 'Medium', color: Color(0xFFC88C61)),
  (label: 'Medium-dark', color: Color(0xFFA46134)),
  (label: 'Dark', color: Color(0xFF5D4437)),
];

int _validSkinTone(int? value) =>
    value != null && value >= 0 && value < _skinTones.length ? value : 0;

/// Category selector: one icon per section of the continuous grid, in scroll
/// order. Tapping jumps to that section; scrolling moves the highlight.
///
/// The icons divide the full content width evenly — the same width the search
/// field above spans — so the rail reads as a segmented control rather than a
/// short left-aligned strip.
class _CategoryRail extends StatelessWidget {
  final List<_EmojiSection> sections;
  final int activeIndex;
  final ValueChanged<int> onSelect;
  final int skinTone;
  final ValueChanged<int> onSkinToneChanged;

  const _CategoryRail({
    required this.sections,
    required this.activeIndex,
    required this.onSelect,
    required this.skinTone,
    required this.onSkinToneChanged,
  });

  @override
  Widget build(BuildContext context) {
    return SizedBox(
      height: _railHeight,
      child: Padding(
        padding: const EdgeInsets.symmetric(horizontal: Grid.gutter),
        child: Row(
          children: [
            for (var i = 0; i < sections.length; i++)
              Expanded(
                child: _CategoryIcon(
                  icon: sections[i].icon,
                  tooltip: sections[i].label,
                  selected: i == activeIndex,
                  onTap: () => onSelect(i),
                ),
              ),
            Expanded(
              child: _SkinToneSelector(
                value: skinTone,
                onChanged: onSkinToneChanged,
              ),
            ),
          ],
        ),
      ),
    );
  }
}

class _SkinToneSelector extends StatelessWidget {
  const _SkinToneSelector({required this.value, required this.onChanged});

  final int value;
  final ValueChanged<int> onChanged;

  @override
  Widget build(BuildContext context) {
    final selected = _skinTones[_validSkinTone(value)];
    return PopupMenuButton<int>(
      key: const ValueKey('emoji-skin-tone-selector'),
      initialValue: value,
      tooltip: 'Skin tone',
      position: PopupMenuPosition.under,
      onSelected: onChanged,
      itemBuilder: (context) => [
        for (final (index, tone) in _skinTones.indexed)
          PopupMenuItem<int>(
            key: ValueKey('emoji-skin-tone-$index'),
            value: index,
            child: Row(
              children: [
                _SkinToneDot(
                  key: ValueKey('emoji-skin-tone-dot-$index'),
                  color: tone.color,
                ),
                const SizedBox(width: Grid.xs),
                Expanded(child: Text(tone.label)),
                if (index == value)
                  Icon(
                    LucideIcons.check,
                    size: 18,
                    color: context.colors.primary,
                  ),
              ],
            ),
          ),
      ],
      child: Semantics(
        button: true,
        label: 'Skin tone',
        child: Center(
          child: _SkinToneDot(
            key: const ValueKey('emoji-skin-tone-dot-selected'),
            color: selected.color,
          ),
        ),
      ),
    );
  }
}

class _SkinToneDot extends StatelessWidget {
  const _SkinToneDot({super.key, required this.color});

  final Color color;

  @override
  Widget build(BuildContext context) {
    return SizedBox.square(
      dimension: 16,
      child: Stack(
        fit: StackFit.expand,
        children: [
          DecoratedBox(
            decoration: BoxDecoration(color: color, shape: BoxShape.circle),
          ),
          ClipOval(
            child: DecoratedBox(
              decoration: BoxDecoration(
                gradient: LinearGradient(
                  colors: [
                    Colors.white.withValues(alpha: 0.2),
                    Colors.transparent,
                  ],
                  begin: Alignment.topCenter,
                  end: Alignment.bottomCenter,
                ),
              ),
            ),
          ),
          DecoratedBox(
            decoration: BoxDecoration(
              shape: BoxShape.circle,
              border: Border.all(color: Colors.black.withValues(alpha: 0.8)),
            ),
          ),
        ],
      ),
    );
  }
}

class _CategoryIcon extends StatelessWidget {
  final IconData icon;
  final String tooltip;
  final bool selected;
  final VoidCallback onTap;

  const _CategoryIcon({
    required this.icon,
    required this.tooltip,
    required this.selected,
    required this.onTap,
  });

  @override
  Widget build(BuildContext context) {
    final colors = context.colors;
    return Tooltip(
      message: tooltip,
      child: InkWell(
        onTap: onTap,
        borderRadius: BorderRadius.circular(Radii.sm),
        child: Semantics(
          button: true,
          selected: selected,
          label: tooltip,
          child: Center(
            child: Icon(
              icon,
              size: 18,
              color: selected ? colors.primary : colors.onSurfaceVariant,
            ),
          ),
        ),
      ),
    );
  }
}
