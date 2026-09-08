import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../shared/emoji/emoji_avatar.dart';
import '../../shared/theme/theme.dart';

/// Vertical preview shift shared by background editors.
const avatarBackgroundPreviewShift = 136.0;

/// The shared background-color grid used by emoji and animated avatars.
class AvatarBackgroundGrid extends StatelessWidget {
  /// Creates a fixed grid of avatar background colors.
  const AvatarBackgroundGrid({
    super.key,
    required this.selectedColor,
    required this.onColorSelected,
    this.colorKeyPrefix = 'avatar-background-color',
  });

  /// The ARGB value of the currently selected background color.
  final int selectedColor;

  /// Called with the selected ARGB value when the user taps a color.
  final ValueChanged<int> onColorSelected;

  /// Prefix used for each color option's test key.
  final String colorKeyPrefix;

  @override
  Widget build(BuildContext context) => GridView.builder(
    shrinkWrap: true,
    primary: false,
    padding: EdgeInsets.zero,
    gridDelegate: const SliverGridDelegateWithFixedCrossAxisCount(
      crossAxisCount: 6,
      crossAxisSpacing: Grid.xxs,
      mainAxisSpacing: Grid.xs,
    ),
    itemCount: emojiAvatarColors.length,
    itemBuilder: (context, index) {
      final color = emojiAvatarColors[index];
      final isSelected = color == selectedColor;
      return Center(
        child: Semantics(
          selected: isSelected,
          label: 'Avatar color ${index + 1}',
          child: InkWell(
            key: ValueKey('$colorKeyPrefix-$index'),
            borderRadius: BorderRadius.circular(Radii.full),
            onTap: () {
              unawaited(HapticFeedback.selectionClick());
              onColorSelected(color);
            },
            child: Container(
              width: 52,
              height: 52,
              padding: const EdgeInsets.all(5),
              decoration: BoxDecoration(
                color: Color(color),
                shape: BoxShape.circle,
                border: Border.all(color: context.colors.outlineVariant),
              ),
              child: isSelected
                  ? DecoratedBox(
                      decoration: BoxDecoration(
                        shape: BoxShape.circle,
                        border: Border.all(
                          color: _selectionColor(Color(color)),
                          width: 3,
                        ),
                      ),
                    )
                  : null,
            ),
          ),
        ),
      );
    },
  );
}

Color _selectionColor(Color color) =>
    color.computeLuminance() > 0.55 ? const Color(0xFF111111) : Colors.white;
