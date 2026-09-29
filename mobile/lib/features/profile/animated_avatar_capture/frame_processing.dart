part of '../animated_avatar_capture.dart';

/// Creates an isolated workspace for one capture's segmentation frames.
@visibleForTesting
Future<Directory> createAnimatedAvatarFrameDirectory({
  Directory? parent,
}) async => (parent ?? await getTemporaryDirectory()).createTemp(
  'buzz-avatar-capture-',
);

Future<List<Uint8List>> _removeBackgrounds(List<Uint8List> frames) async {
  final segmenter = SelfieSegmenter(
    mode: SegmenterMode.stream,
    enableRawSizeMask: false,
  );
  Directory? captureDirectory;
  final results = <Uint8List>[];
  try {
    captureDirectory = await createAnimatedAvatarFrameDirectory();
    for (var index = 0; index < frames.length; index++) {
      final file = File('${captureDirectory.path}/frame-$index.png');
      await file.writeAsBytes(frames[index], flush: false);
      final mask = await segmenter.processImage(
        InputImage.fromFilePath(file.path),
      );
      if (mask == null) {
        results.add(frames[index]);
        continue;
      }
      results.add(
        await compute(
          _applySegmentationMask,
          _MaskRequest(
            frame: frames[index],
            maskWidth: mask.width,
            maskHeight: mask.height,
            confidences: Float32List.fromList(mask.confidences),
          ),
        ),
      );
    }
  } finally {
    await segmenter.close();
    final directory = captureDirectory;
    if (directory != null && await directory.exists()) {
      try {
        await directory.delete(recursive: true);
      } on FileSystemException {
        // Temporary capture cleanup is best effort.
      }
    }
  }
  return results;
}

@immutable
class _MaskRequest {
  const _MaskRequest({
    required this.frame,
    required this.maskWidth,
    required this.maskHeight,
    required this.confidences,
  });

  final Uint8List frame;
  final int maskWidth;
  final int maskHeight;
  final Float32List confidences;
}

Uint8List _applySegmentationMask(_MaskRequest request) {
  final result = image.decodePng(request.frame)!.convert(numChannels: 4);
  for (var y = 0; y < result.height; y++) {
    final maskY = (y * request.maskHeight / result.height).floor().clamp(
      0,
      request.maskHeight - 1,
    );
    for (var x = 0; x < result.width; x++) {
      final maskX = (x * request.maskWidth / result.width).floor().clamp(
        0,
        request.maskWidth - 1,
      );
      final confidence = request.confidences[maskY * request.maskWidth + maskX];
      final alpha = ((confidence - 0.28) / (0.72 - 0.28)).clamp(0, 1);
      final pixel = result.getPixel(x, y);
      pixel.a = (alpha * 255).round();
    }
  }
  return image.encodePng(result, level: 4);
}

/// Encodes one poster frame for validating animated-avatar framing parity.
@visibleForTesting
Uint8List encodeAnimatedAvatarPoster({
  required Uint8List frame,
  required double scale,
}) => _encodeAvatar(
  _EncodeRequest(
    frames: [frame],
    posterIndex: 0,
    scale: scale,
    offsetX: 0,
    offsetY: 0,
    backdropColor: 0xff0000ff,
    personOutline: false,
    shapeScale: 1,
    shapeOffsetX: 0,
    shapeOffsetY: 0,
  ),
).poster;
