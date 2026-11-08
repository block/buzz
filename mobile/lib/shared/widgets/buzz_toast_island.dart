part of 'buzz_toast.dart';

// Portrait Dynamic Island iPhones expose a top safe area above 50 points.
// Read the live inset so an early cold-start window cannot hide the effect.
Rect? _toastIslandOrigin(BuildContext context) {
  final media = MediaQuery.of(context);
  if (defaultTargetPlatform != TargetPlatform.iOS ||
      media.orientation != Orientation.portrait ||
      media.size.shortestSide >= 600 ||
      media.viewPadding.top <= 50) {
    return null;
  }
  return Rect.fromLTWH(
    (media.size.width - 120) / 2,
    11 + (media.viewPadding.top - 59).clamp(0.0, double.infinity),
    120,
    36,
  );
}

Future<ui.FragmentProgram?>? _islandProgram;

Future<ui.FragmentProgram?> _loadIslandProgram() =>
    _islandProgram ??= () async {
      try {
        return await ui.FragmentProgram.fromAsset('shaders/toast_island.frag');
      } catch (error) {
        debugPrint('Could not load toast island effect: $error');
        return null;
      }
    }();

// Paint the island and moving capsule together. Text/actions are a separate,
// sharp layer, with no shadow or opaque capsule dividing the merged surface.
class _ToastIslandSurface extends CustomPainter {
  const _ToastIslandSurface({
    required this.shader,
    required this.island,
    required this.top,
    required this.progress,
    required this.reveal,
    required this.pixelRatio,
  });

  final ui.FragmentShader shader;
  final Rect island;
  final double top;
  final double progress;
  final double reveal;
  final double pixelRatio;

  @override
  void paint(Canvas canvas, Size size) {
    final bubbleTop =
        top + (island.center.dy - top - size.height / 2) * (1 - progress);
    final localIsland = Rect.fromCenter(
      center: Offset(size.width / 2, island.center.dy - bubbleTop),
      width: island.width,
      height: island.height,
    );
    final t = reveal.clamp(0.0, 1.0);
    final travel = progress.clamp(0.0, 1.0);
    final toast = Rect.fromCenter(
      center: size.center(Offset.zero),
      // Start compact even when the message is narrower than the island.
      // The stationary island keeps its own full-width outline above.
      width: island.height.clamp(0.0, size.width) * (1 - t) + size.width * t,
      height: island.height * (1 - t) + size.height * t,
    );
    final bounds = localIsland.expandToInclude(toast).inflate(24);
    final origin = bounds.topLeft;
    final values = [
      localIsland.center.dx - origin.dx,
      localIsland.center.dy - origin.dy,
      localIsland.width / 2,
      localIsland.height / 2,
      toast.center.dx - origin.dx,
      toast.center.dy - origin.dy,
      toast.width / 2,
      toast.height / 2,
      // Follow travel, not width: the outro narrows early but must keep
      // merging until it actually reaches the island. Endpoints stay flat.
      128 * travel * (1 - travel),
      0.75 / pixelRatio,
    ];
    for (var i = 0; i < values.length; i++) {
      shader.setFloat(i, values[i]);
    }
    canvas.save();
    canvas.translate(origin.dx, origin.dy);
    canvas.drawRect(Offset.zero & bounds.size, Paint()..shader = shader);
    canvas.restore();
  }

  @override
  bool shouldRepaint(_ToastIslandSurface oldDelegate) =>
      shader != oldDelegate.shader ||
      island != oldDelegate.island ||
      top != oldDelegate.top ||
      progress != oldDelegate.progress ||
      reveal != oldDelegate.reveal ||
      pixelRatio != oldDelegate.pixelRatio;
}
