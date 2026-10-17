part of '../channels_page.dart';

class _CommunityGridAction extends HookWidget {
  const _CommunityGridAction({
    required this.community,
    required this.isActive,
    required this.isEditing,
    required this.onRemove,
  });

  final Community community;
  final bool isActive;
  final bool isEditing;
  final VoidCallback onRemove;

  @override
  Widget build(BuildContext context) {
    final reducedMotion = MediaQuery.disableAnimationsOf(context);
    final controller = useAnimationController(
      duration: const Duration(milliseconds: 220),
      initialValue: isEditing ? 1 : 0,
    );
    useEffect(() {
      if (reducedMotion) {
        controller.value = isEditing ? 1 : 0;
      } else if (isEditing) {
        unawaited(controller.forward());
      } else {
        unawaited(controller.reverse());
      }
      return null;
    }, [isEditing, reducedMotion]);
    final progress = useAnimation(controller);
    final t = Curves.easeInOutCubic.transform(progress);
    final size = isActive ? lerpDouble(24, 36, t)! : 36.0;
    final foreground = isActive
        ? Color.lerp(Colors.white, context.colors.onErrorContainer, t)!
        : context.colors.onErrorContainer;

    // A fixed touch target and center keep Edit from nudging the avatar/check.
    return SizedBox.square(
      key: Key('community-switcher-action-${community.id}'),
      dimension: Grid.xl,
      child: IconButton(
        key: Key(
          'community-switcher-${isEditing ? 'remove' : 'selection'}-${community.id}',
        ),
        tooltip: isEditing ? 'Remove ${community.name}' : null,
        padding: EdgeInsets.zero,
        onPressed: isEditing ? onRemove : null,
        icon: Opacity(
          key: Key('community-switcher-action-opacity-${community.id}'),
          opacity: isActive ? 1 : t,
          child: Transform.scale(
            key: Key('community-switcher-action-scale-${community.id}'),
            scale: isActive ? 1 : lerpDouble(.9, 1, t)!,
            child: Container(
              key: Key('community-switcher-badge-${community.id}'),
              width: size,
              height: size,
              decoration: BoxDecoration(
                shape: BoxShape.circle,
                color: isActive
                    ? Color.lerp(
                        context.appColors.success,
                        context.colors.errorContainer,
                        t,
                      )
                    : context.colors.errorContainer,
                border: Border.all(color: context.colors.surface, width: 2),
              ),
              child: Center(
                child: isActive && t > 0 && t < 1
                    ? CustomPaint(
                        size: Size.square(lerpDouble(14, 18, t)!),
                        painter: _CommunityActionGlyphPainter(t, foreground),
                      )
                    : isActive && t == 0
                    ? Icon(BuzzIcons.check, color: foreground, size: 14)
                    : t > 0
                    ? Icon(BuzzIcons.trash2, color: foreground, size: 18)
                    : const SizedBox.shrink(),
              ),
            ),
          ),
        ),
      ),
    );
  }
}

/// Interpolates the check stroke into the bin outline; its lid and ribs appear
/// as the outline opens. Reversing Edit follows the same geometry backwards.
class _CommunityActionGlyphPainter extends CustomPainter {
  const _CommunityActionGlyphPainter(this.progress, this.color);
  final double progress;
  final Color color;

  @override
  void paint(Canvas canvas, Size size) {
    canvas.scale(size.width / 24, size.height / 24);
    final paint = Paint()
      ..color = color
      ..style = PaintingStyle.stroke
      ..strokeWidth = 2
      ..strokeCap = StrokeCap.round
      ..strokeJoin = StrokeJoin.round;
    Offset point(Offset check, Offset trash) =>
        Offset.lerp(check, trash, progress)!;
    final start = point(const Offset(4, 12), const Offset(5, 6));
    final path = Path()..moveTo(start.dx, start.dy);
    void line(Offset check, Offset trash) {
      final p = point(check, trash);
      path.lineTo(p.dx, p.dy);
    }

    void curve(
      Offset checkControl,
      Offset checkEnd,
      Offset trashControl,
      Offset trashEnd,
    ) {
      final c = point(checkControl, trashControl);
      final p = point(checkEnd, trashEnd);
      path.quadraticBezierTo(c.dx, c.dy, p.dx, p.dy);
    }

    line(const Offset(7, 15), const Offset(5, 20));
    curve(
      const Offset(8, 16),
      const Offset(9, 17),
      const Offset(5, 22),
      const Offset(7, 22),
    );
    line(const Offset(14.5, 11.5), const Offset(17, 22));
    curve(
      const Offset(17.25, 8.75),
      const Offset(20, 6),
      const Offset(19, 22),
      const Offset(19, 20),
    );
    line(const Offset(20, 6), const Offset(19, 6));
    canvas.drawPath(path, paint);
    paint.color = color.withValues(alpha: progress);
    canvas.drawPath(
      Path()
        ..moveTo(3, 6)
        ..lineTo(21, 6)
        ..moveTo(9, 6)
        ..lineTo(9, 4)
        ..quadraticBezierTo(9, 2, 11, 2)
        ..lineTo(13, 2)
        ..quadraticBezierTo(15, 2, 15, 4)
        ..lineTo(15, 6)
        ..moveTo(10, 10)
        ..lineTo(10, 16)
        ..moveTo(14, 10)
        ..lineTo(14, 16),
      paint,
    );
  }

  @override
  bool shouldRepaint(_CommunityActionGlyphPainter oldDelegate) =>
      oldDelegate.progress != progress || oldDelegate.color != color;
}
