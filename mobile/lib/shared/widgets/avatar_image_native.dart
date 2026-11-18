part of 'avatar_image.dart';

/// Renders the same avatar sources used by Flutter as a PNG for native bars.
Future<Uint8List?> nativeAvatarImage({
  required String? url,
  ValueListenable<bool>? cancelled,
  required String initial,
  bool isAgent = false,
  bool circular = false,
  double dimension = 72,
  required Color background,
  required Color foreground,
  required List<ImageProvider> Function(String url) networkImages,
}) async {
  final animatedAvatar = parseAnimatedAvatarUrl(url);
  final poster = animatedAvatar?.posterUrl ?? url;
  final source = _AvatarSource.parse(poster);
  final recorder = ui.PictureRecorder();
  final canvas = Canvas(recorder);
  final bounds = Rect.fromLTWH(0, 0, dimension, dimension);
  canvas.clipPath(avatarPath(bounds, isAgent: isAgent && !circular));
  canvas.drawColor(background, BlendMode.src);
  final imageBounds = isAgent && usesInsetAgentArtwork(url)
      ? bounds.deflate(dimension * (1 - animatedAvatarArtworkScale) / 2)
      : bounds;
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
            rect: imageBounds,
            image: frame.image,
            fit: BoxFit.cover,
          );
          frame.image.dispose();
        } finally {
          codec.dispose();
        }
        rendered = true;
      case _NetworkAvatarSource(:final url):
        final providers = networkImages(url);
        for (var i = 0; i < providers.length; i++) {
          final ImageInfo image;
          try {
            image = await _loadNativeAvatar(
              providers[i],
              dimension.toInt(),
              cancelled,
            );
          } on NetworkImageLoadException catch (error) {
            // A missing relay thumbnail moves on to the next one.
            if (error.statusCode == 404 && i < providers.length - 1) continue;
            rethrow;
          }
          paintImage(
            canvas: canvas,
            rect: imageBounds,
            image: image.image,
            fit: BoxFit.cover,
          );
          image.dispose();
          rendered = true;
          break;
        }
      case null:
        break;
    }
  } catch (_) {
    // Match the normal avatar's initial fallback when media is unavailable.
  }
  if (cancelled?.value == true) {
    recorder.endRecording().dispose();
    return null;
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

Future<ImageInfo> _loadNativeAvatar(
  ImageProvider provider,
  int dimension,
  ValueListenable<bool>? cancelled,
) async {
  if (cancelled?.value == true) throw StateError('Avatar load cancelled');
  final resized = ResizeImage(
    provider,
    width: dimension,
    height: dimension,
    policy: ResizeImagePolicy.fit,
  );
  final stream = resized.resolve(ImageConfiguration.empty);
  final completer = Completer<ImageInfo?>();
  var displayExpired = false;
  var settled = false;
  late final ImageStreamListener listener;
  late final VoidCallback onCancelled;
  void finish({bool abandon = false}) {
    if (settled) return;
    settled = true;
    cancelled?.removeListener(onCancelled);
    stream.removeListener(listener);
    if (abandon) abandonPendingImage(stream, resized, ImageConfiguration.empty);
  }

  onCancelled = () {
    if (cancelled?.value != true || settled) return;
    completer.completeError(StateError('Avatar load cancelled'));
    finish(abandon: true);
  };
  listener = ImageStreamListener(
    (image, _) {
      if (settled) return;
      completer.complete(displayExpired ? null : image.clone());
      finish();
    },
    onError: (Object error, StackTrace? stack) {
      if (settled) return;
      completer.completeError(error, stack);
      finish();
    },
  );
  cancelled?.addListener(onCancelled);
  stream.addListener(listener);
  // This bounds how long the UI waits, not the network request. Keep listening
  // until the load settles so a later sheet dismissal can still cancel it.
  final image = await completer.future.timeout(
    const Duration(seconds: 10),
    onTimeout: () {
      displayExpired = true;
      return null;
    },
  );
  if (image == null) throw TimeoutException('Avatar display wait expired');
  return image;
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
