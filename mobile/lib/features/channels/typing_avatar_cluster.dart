import 'package:flutter/material.dart';
import '../../shared/theme/theme.dart';

/// Fixed message-avatar footprint with at most four animated occupants.
class TypingAvatarCluster extends StatelessWidget {
  const TypingAvatarCluster({
    super.key,
    required this.count,
    required this.avatars,
  });
  final int count;
  final Map<String, Widget> avatars;

  @override
  Widget build(BuildContext context) {
    final duration = MediaQuery.disableAnimationsOf(context)
        ? Duration.zero
        : Duration(milliseconds: count == 3 || count == 4 ? 420 : 360);
    final size = switch (count) {
      1 => messageAvatarSize,
      2 => 28.0,
      3 => 26.0,
      _ => 24.0,
    };
    final edge = messageAvatarSize - size;
    const badgeSize = 26.0;
    final positions = switch (count) {
      1 => [Offset.zero],
      2 => [Offset.zero, Offset(edge, edge)],
      3 => [Offset(edge / 2, 0), Offset(0, edge), Offset(edge, edge)],
      _ => [Offset.zero, Offset(edge, 0), Offset(0, edge), Offset(edge, edge)],
    };
    final children = avatars.entries.take(count > 4 ? 3 : 4).toList();
    return SizedBox(
      key: const ValueKey('typing-avatar-cluster'),
      width: messageAvatarSize,
      height: messageAvatarSize,
      child: Stack(
        clipBehavior: Clip.none,
        children: [
          for (var i = 0; i < children.length; i++)
            TweenAnimationBuilder<Rect>(
              key: ValueKey(children[i].key),
              tween: _AvatarRectTween(
                end: Rect.fromLTWH(
                  positions[i].dx,
                  positions[i].dy,
                  size,
                  size,
                ),
              ),
              duration: duration,
              curve: count == 2 ? Curves.easeOutBack : Curves.easeOutCubic,
              builder: (context, rect, child) =>
                  Positioned.fromRect(rect: rect, child: child!),
              child: _arrival(
                duration: count == 1 ? Duration.zero : duration,
                offset: Offset(
                  positions[i].dx < edge / 2 ? -12 : 12,
                  positions[i].dy < edge / 2 ? -8 : 8,
                ),
                child: FittedBox(child: children[i].value),
              ),
            ),
          if (count > 4)
            Positioned(
              left: messageAvatarSize - badgeSize,
              top: messageAvatarSize - badgeSize,
              width: badgeSize,
              height: badgeSize,
              child: _arrival(
                duration: duration,
                offset: const Offset(12, 8),
                child: Container(
                  key: const ValueKey('typing-avatar-overflow'),
                  decoration: BoxDecoration(
                    color: context.colors.secondaryContainer,
                    shape: BoxShape.circle,
                  ),
                  padding: const EdgeInsets.all(2),
                  child: Center(
                    child: FittedBox(
                      fit: BoxFit.scaleDown,
                      child: Text(
                        '+${count - 3}',
                        style: context.textTheme.labelSmall?.copyWith(
                          fontWeight: FontWeight.w700,
                          fontSize: 12,
                          color: context.colors.onSecondaryContainer,
                        ),
                      ),
                    ),
                  ),
                ),
              ),
            ),
        ],
      ),
    );
  }

  Widget _arrival({
    required Duration duration,
    required Offset offset,
    required Widget child,
  }) {
    return TweenAnimationBuilder<double>(
      tween: Tween(begin: 0, end: 1),
      duration: duration,
      curve: count == 2 ? Curves.easeOutBack : Curves.easeOutCubic,
      builder: (context, value, child) => Opacity(
        opacity: value.clamp(0.0, 1.0),
        child: Transform.translate(
          offset: offset * (1 - value),
          child: Transform.scale(scale: 0.95 + 0.05 * value, child: child),
        ),
      ),
      child: child,
    );
  }
}

/// Interpolate directly between layouts, keeping the first frame stationary.
class _AvatarRectTween extends Tween<Rect> {
  _AvatarRectTween({required Rect end}) : super(begin: end, end: end);

  @override
  Rect lerp(double t) => Rect.lerp(begin, end, t)!;
}
