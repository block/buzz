part of 'avatar_image.dart';

/// Renders the same avatar sources used by Flutter as a PNG for native bars.
Future<Uint8List?> nativeAvatarImage({
  required String? url,
  required String initial,
  bool isAgent = false,
  required Color background,
  required Color foreground,
  required ImageProvider Function(String) networkImage,
}) async {
  const dimension = 72.0;
  final poster = parseAnimatedAvatarUrl(url)?.posterUrl ?? url;
  final source = _AvatarSource.parse(poster);
  final recorder = ui.PictureRecorder();
  final canvas = Canvas(recorder);
  final bounds = const Rect.fromLTWH(0, 0, dimension, dimension);
  canvas.clipRRect(
    RRect.fromRectAndRadius(
      bounds,
      Radius.circular(dimension * (isAgent ? 0.3 : 0.5)),
    ),
  );
  canvas.drawColor(background, BlendMode.src);
  var rendered = false;
  try {
    switch (source) {
      case _EmojiAvatarSource(:final emoji, :final color):
        canvas.drawColor(color, BlendMode.src);
        await _paintNativeAvatarEmoji(
          canvas,
          emoji,
          dimension * 258 / 512,
          foreground,
        );
        rendered = true;
      case _SvgAvatarSource(:final svg):
        final picture = await vg.loadPicture(SvgStringLoader(svg), null);
        final scale = dimension / picture.size.shortestSide;
        canvas.save();
        canvas.translate(
          (dimension - picture.size.width * scale) / 2,
          (dimension - picture.size.height * scale) / 2,
        );
        canvas.scale(scale);
        canvas.drawPicture(picture.picture);
        canvas.restore();
        picture.picture.dispose();
        rendered = true;
      case _RasterDataAvatarSource(:final bytes):
        final buffer = await ui.ImmutableBuffer.fromUint8List(bytes);
        final codec = await ui.instantiateImageCodecWithSize(
          buffer,
          getTargetSize: (width, height) => width >= height
              ? ui.TargetImageSize(width: width.clamp(1, dimension.toInt()))
              : ui.TargetImageSize(height: height.clamp(1, dimension.toInt())),
        );
        try {
          final frame = await codec.getNextFrame();
          paintImage(
            canvas: canvas,
            rect: bounds,
            image: frame.image,
            fit: BoxFit.cover,
          );
          frame.image.dispose();
        } finally {
          codec.dispose();
        }
        rendered = true;
      case _NetworkAvatarSource(:final url):
        final stream = ResizeImage(
          networkImage(url),
          width: dimension.toInt(),
          height: dimension.toInt(),
          policy: ResizeImagePolicy.fit,
        ).resolve(ImageConfiguration.empty);
        final completer = Completer<ImageInfo>();
        final listener = ImageStreamListener(
          (image, _) {
            if (!completer.isCompleted) completer.complete(image.clone());
          },
          onError: (Object error, StackTrace? stack) {
            if (!completer.isCompleted) completer.completeError(error, stack);
          },
        );
        stream.addListener(listener);
        try {
          final image = await completer.future.timeout(
            const Duration(seconds: 10),
          );
          paintImage(
            canvas: canvas,
            rect: bounds,
            image: image.image,
            fit: BoxFit.cover,
          );
          image.dispose();
          rendered = true;
        } finally {
          stream.removeListener(listener);
        }
      case null:
        break;
    }
  } catch (_) {
    // Match the normal avatar's initial fallback when media is unavailable.
  }
  if (!rendered) _paintNativeAvatarText(canvas, initial, 32, foreground);
  final picture = recorder.endRecording();
  final image = await picture.toImage(dimension.toInt(), dimension.toInt());
  picture.dispose();
  try {
    return (await image.toByteData(
      format: ui.ImageByteFormat.png,
    ))?.buffer.asUint8List();
  } finally {
    image.dispose();
  }
}

void _paintNativeAvatarText(
  Canvas canvas,
  String text,
  double size,
  Color color,
) {
  final painter = TextPainter(
    text: TextSpan(
      text: text,
      style: TextStyle(fontSize: size, color: color, height: 1),
    ),
    textDirection: TextDirection.ltr,
  )..layout();
  painter.paint(
    canvas,
    Offset((72 - painter.width) / 2, (72 - painter.height) / 2),
  );
  painter.dispose();
}

// Text advances and baselines are not the visible bounds of a color emoji.
// Rasterize on a padded transparent surface, then center the painted pixels.
Future<void> _paintNativeAvatarEmoji(
  Canvas canvas,
  String emoji,
  double size,
  Color color,
) async {
  final painter = TextPainter(
    text: TextSpan(
      text: emoji,
      style: TextStyle(fontSize: size, color: color),
    ),
    textDirection: TextDirection.ltr,
  )..layout();
  final width = (painter.width + size * 2).ceil();
  final height = (painter.height + size * 2).ceil();
  final recorder = ui.PictureRecorder();
  painter.paint(Canvas(recorder), Offset(size, size));
  painter.dispose();
  final picture = recorder.endRecording();
  final image = await picture.toImage(width, height);
  picture.dispose();
  try {
    final pixels = await image.toByteData(format: ui.ImageByteFormat.rawRgba);
    if (pixels == null) return;
    var left = width;
    var top = height;
    var right = -1;
    var bottom = -1;
    for (var y = 0; y < height; y++) {
      for (var x = 0; x < width; x++) {
        if (pixels.getUint8((y * width + x) * 4 + 3) < 8) continue;
        if (x < left) left = x;
        if (x > right) right = x;
        if (y < top) top = y;
        if (y > bottom) bottom = y;
      }
    }
    if (right < left || bottom < top) return;
    final ink = Rect.fromLTRB(
      left.toDouble(),
      top.toDouble(),
      right + 1.0,
      bottom + 1.0,
    );
    final longest = ink.width > ink.height ? ink.width : ink.height;
    final scale = (size / longest).clamp(0.0, 1.0);
    canvas.drawImageRect(
      image,
      ink,
      Rect.fromCenter(
        center: const Offset(36, 36),
        width: ink.width * scale,
        height: ink.height * scale,
      ),
      Paint()..filterQuality = FilterQuality.high,
    );
  } finally {
    image.dispose();
  }
}
