part of '../animated_avatar_capture.dart';

class _ErrorText extends StatelessWidget {
  const _ErrorText(this.message);

  final String message;

  @override
  Widget build(BuildContext context) => Padding(
    padding: const EdgeInsets.only(top: Grid.xs),
    child: Semantics(
      liveRegion: true,
      child: Text(
        message,
        textAlign: TextAlign.center,
        style: context.textTheme.bodySmall?.copyWith(
          color: context.colors.error,
        ),
      ),
    ),
  );
}

Color _personOutlineColor(int backdropColor) {
  final color = Color(backdropColor);
  return color.computeLuminance() > 0.74
      ? const Color(0xFF111111)
      : Colors.white;
}
