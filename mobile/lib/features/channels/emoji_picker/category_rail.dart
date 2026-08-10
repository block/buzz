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

  const _CategoryRail({
    required this.sections,
    required this.activeIndex,
    required this.onSelect,
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
          ],
        ),
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
