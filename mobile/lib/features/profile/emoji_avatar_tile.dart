import 'package:flutter/material.dart';

import '../../shared/emoji/native_emoji_glyph.dart';
import '../../shared/theme/theme.dart';

/// A selectable emoji tile with an explicit accessibility selection state.
class EmojiAvatarTile extends StatelessWidget {
  const EmojiAvatarTile({
    required this.emoji,
    required this.label,
    required this.tileId,
    required this.isSelected,
    required this.onTap,
    super.key,
  });

  final String emoji;
  final String label;
  final String tileId;
  final bool isSelected;
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
