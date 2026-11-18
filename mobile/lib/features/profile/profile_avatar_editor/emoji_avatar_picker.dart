part of '../profile_avatar_editor.dart';

const _emojiPreviewGlyphSize = _previewSize * 258 / 512;

class _EmojiAvatarPreview extends StatelessWidget {
  const _EmojiAvatarPreview({
    super.key,
    required this.emoji,
    required this.color,
    required this.animationKey,
    required this.reduceMotion,
  });

  final String emoji;
  final Color color;
  final int animationKey;
  final bool reduceMotion;

  @override
  Widget build(BuildContext context) => AnimatedContainer(
    key: const ValueKey('emoji-avatar-preview'),
    width: _previewSize,
    height: _previewSize,
    duration: reduceMotion ? Duration.zero : _previewSquishDuration,
    curve: _entranceCurve,
    decoration: BoxDecoration(color: color, shape: BoxShape.circle),
    child: Center(
      child: TweenAnimationBuilder<double>(
        key: ValueKey('emoji-avatar-preview-glyph-$animationKey'),
        tween: Tween(begin: 0.88, end: 1),
        duration: reduceMotion ? Duration.zero : _previewSquishDuration,
        curve: _entranceCurve,
        builder: (context, scaleY, child) => Transform.scale(
          scaleX: 0.96 + ((scaleY - 0.88) / 0.12) * 0.04,
          scaleY: scaleY,
          child: child,
        ),
        child: NativeEmojiGlyph(
          emoji: emoji,
          size: _emojiPreviewGlyphSize,
          opticalBoxSize: _emojiPreviewGlyphSize,
        ),
      ),
    ),
  );
}
