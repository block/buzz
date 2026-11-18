import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/emoji/emoji_avatar.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/navigation_button_surface.dart';
import 'agent_avatar_geometry.dart';
import 'agent_category_motion.dart';

/// Emoji preview opens the system picker; background colors stay in the tray.
class AgentEmojiStage extends HookWidget {
  const AgentEmojiStage({
    super.key,
    required this.height,
    required this.preview,
    required this.emoji,
    required this.selectedColor,
    required this.disabled,
    required this.onChooseEmoji,
    required this.onColorSelected,
  });

  final double height;
  final Widget preview;
  final String? emoji;
  final int selectedColor;
  final bool disabled;
  final VoidCallback onChooseEmoji;
  final ValueChanged<int> onColorSelected;

  @override
  Widget build(BuildContext context) {
    final showHint = useState(true);
    final scroll = useScrollController();
    final reduceMotion = MediaQuery.disableAnimationsOf(context);
    final pop = useAnimationController(
      duration: const Duration(milliseconds: 280),
      initialValue: 1,
    );
    final previousEmoji = useRef(emoji);
    useEffect(() {
      if (previousEmoji.value != emoji && emoji != null) {
        if (reduceMotion) {
          pop.value = 1;
        } else {
          pop.forward(from: 0);
        }
      }
      previousEmoji.value = emoji;
      return null;
    }, [emoji, reduceMotion]);
    return SizedBox(
      height: height,
      child: Stack(
        children: [
          Positioned.fill(
            bottom: agentAvatarTraySpace,
            child: AgentCategoryMotion(
              child: Center(
                child: Semantics(
                  label: 'Choose emoji',
                  hint: 'Tap to change emoji',
                  button: true,
                  child: GestureDetector(
                    key: const ValueKey('agent-choose-emoji'),
                    onTap: disabled
                        ? null
                        : () {
                            showHint.value = false;
                            onChooseEmoji();
                          },
                    child: Stack(
                      clipBehavior: Clip.none,
                      children: [
                        AnimatedBuilder(
                          animation: pop,
                          child: preview,
                          builder: (context, child) => Transform.scale(
                            scale: reduceMotion
                                ? 1
                                : 0.92 +
                                      0.08 *
                                          Curves.easeOutBack.transform(
                                            pop.value,
                                          ),
                            child: child,
                          ),
                        ),
                        Positioned(
                          // Point to the glyph inside the 220-point avatar.
                          bottom: 180,
                          left: -20,
                          right: -20,
                          child: IgnorePointer(
                            child: ExcludeSemantics(
                              child: AnimatedOpacity(
                                opacity: showHint.value ? 1 : 0,
                                duration:
                                    MediaQuery.disableAnimationsOf(context)
                                    ? Duration.zero
                                    : const Duration(milliseconds: 150),
                                child: Center(
                                  child: Column(
                                    mainAxisSize: MainAxisSize.min,
                                    children: [
                                      DecoratedBox(
                                        decoration: BoxDecoration(
                                          color: context.colors.inverseSurface,
                                          borderRadius: BorderRadius.circular(
                                            Radii.container,
                                          ),
                                        ),
                                        child: Padding(
                                          padding: const EdgeInsets.symmetric(
                                            horizontal: Grid.xs,
                                            vertical: Grid.xxs,
                                          ),
                                          child: Text(
                                            'Tap to change emoji',
                                            style: context.textTheme.labelMedium
                                                ?.copyWith(
                                                  color: context
                                                      .colors
                                                      .onInverseSurface,
                                                ),
                                          ),
                                        ),
                                      ),
                                      CustomPaint(
                                        size: const Size(20, 8),
                                        painter: _EmojiHintPointer(
                                          context.colors.inverseSurface,
                                        ),
                                      ),
                                    ],
                                  ),
                                ),
                              ),
                            ),
                          ),
                        ),
                      ],
                    ),
                  ),
                ),
              ),
            ),
          ),
          Positioned(
            left: 0,
            right: 0,
            bottom: Grid.sm,
            height: 68,
            child: AgentCategoryMotion(
              slide: true,
              child: NavigationButtonSurface(
                capsule: true,
                child: LayoutBuilder(
                  builder: (context, constraints) => ListView.builder(
                    controller: scroll,
                    key: const ValueKey('agent-emoji-colors'),
                    scrollDirection: Axis.horizontal,
                    padding: const EdgeInsets.symmetric(horizontal: Grid.xxs),
                    itemExtent: 64,
                    itemCount: emojiAvatarColors.length,
                    itemBuilder: (context, index) {
                      final color = Color(emojiAvatarColors[index]);
                      final selected = color.toARGB32() == selectedColor;
                      return Center(
                        child: Semantics(
                          label: 'Avatar color ${index + 1}',
                          button: true,
                          selected: selected,
                          child: InkResponse(
                            key: ValueKey('agent-emoji-color-$index'),
                            onTap: disabled
                                ? null
                                : () {
                                    HapticFeedback.selectionClick();
                                    onColorSelected(color.toARGB32());
                                  },
                            child: AnimatedBuilder(
                              animation: scroll,
                              builder: (context, child) {
                                final scale = agentFilmstripScale(
                                  index: index,
                                  count: emojiAvatarColors.length,
                                  extent: 64,
                                  width: constraints.maxWidth,
                                  offset: scroll.hasClients ? scroll.offset : 0,
                                  reduceMotion: reduceMotion,
                                );
                                return Transform.scale(
                                  scale: scale,
                                  child: child,
                                );
                              },
                              child: Container(
                                width: 48,
                                height: 48,
                                padding: const EdgeInsets.all(5),
                                decoration: BoxDecoration(
                                  color: color,
                                  shape: BoxShape.circle,
                                  border: Border.all(
                                    color: context.colors.outlineVariant,
                                  ),
                                ),
                                child: selected
                                    ? DecoratedBox(
                                        decoration: BoxDecoration(
                                          shape: BoxShape.circle,
                                          border: Border.all(
                                            width: 3,
                                            color:
                                                color.computeLuminance() > .55
                                                ? Colors.black
                                                : Colors.white,
                                          ),
                                        ),
                                      )
                                    : null,
                              ),
                            ),
                          ),
                        ),
                      );
                    },
                  ),
                ),
              ),
            ),
          ),
        ],
      ),
    );
  }
}

/// A curved tip joins the bubble without a sharp triangular point.
class _EmojiHintPointer extends CustomPainter {
  const _EmojiHintPointer(this.color);

  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    final path = Path()
      ..moveTo(0, 0)
      ..cubicTo(5, 0, 7, 8, 10, 8)
      ..cubicTo(13, 8, 15, 0, 20, 0)
      ..close();
    canvas.drawPath(path, Paint()..color = color);
  }

  @override
  bool shouldRepaint(_EmojiHintPointer oldDelegate) =>
      oldDelegate.color != color;
}
