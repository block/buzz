import 'package:flutter/material.dart';

import '../../shared/emoji/native_emoji_glyph.dart';
import '../../shared/theme/theme.dart';

/// A selectable emoji tile with an explicit accessibility selection state.
class EmojiAvatarTile extends StatelessWidget {
  /// Creates an emoji option for an avatar picker.
  const EmojiAvatarTile({
    required this.emoji,
    required this.label,
    required this.tileId,
    required this.isSelected,
    required this.onTap,
    super.key,
  });

  /// The Unicode emoji glyph rendered by this tile.
  final String emoji;

  /// The human-readable emoji name announced to assistive technology.
  final String label;

  /// The stable identifier used for the tile's widget key.
  final String tileId;

  /// Whether this emoji is the current avatar selection.
  final bool isSelected;

  /// Called when the tile is selected.
  final VoidCallback onTap;

  @override
  Widget build(BuildContext context) => Semantics(
    label: label,
    button: true,
    selected: isSelected,
    onTap: onTap,
    child: ExcludeSemantics(
      child: InkWell(
        key: ValueKey('emoji-avatar-$tileId'),
        borderRadius: BorderRadius.circular(Radii.sm),
        onTap: onTap,
        child: DecoratedBox(
          decoration: BoxDecoration(
            color: isSelected
                ? context.colors.primaryContainer
                : Colors.transparent,
            borderRadius: BorderRadius.circular(Radii.sm),
          ),
          child: Center(
            child: NativeEmojiGlyph(emoji: emoji, size: 30, opticalBoxSize: 30),
          ),
        ),
      ),
    ),
  );
}
