part of '../media_upload.dart';

bool _startsWith(Uint8List bytes, List<int> prefix) {
  if (bytes.length < prefix.length) return false;
  for (var i = 0; i < prefix.length; i++) {
    if (bytes[i] != prefix[i]) return false;
  }
  return true;
}

bool _matchesAscii(Uint8List bytes, int offset, String value) {
  final codeUnits = ascii.encode(value);
  if (bytes.length < offset + codeUnits.length) return false;
  for (var i = 0; i < codeUnits.length; i++) {
    if (bytes[offset + i] != codeUnits[i]) return false;
  }
  return true;
}

int _readUint32BigEndian(Uint8List bytes, int offset) {
  return (bytes[offset] << 24) |
      (bytes[offset + 1] << 16) |
      (bytes[offset + 2] << 8) |
      bytes[offset + 3];
}

int _readUint32LittleEndian(Uint8List bytes, int offset) {
  return bytes[offset] |
      (bytes[offset + 1] << 8) |
      (bytes[offset + 2] << 16) |
      (bytes[offset + 3] << 24);
}

Future<Uint8List?> _readPlatformClipboardImage() async {
  return _mediaUploadPlatformChannel.invokeMethod<Uint8List>(
    _readClipboardImageMethod,
  );
}

Future<Uint8List?> _generatePickedVideoPoster(String filePath) {
  return _mediaUploadPlatformChannel.invokeMethod<Uint8List>(
    _generateVideoPosterMethod,
    filePath,
  );
}

Future<String> _transcodePickedVideoToMp4(String filePath) async {
  final result = await _mediaUploadPlatformChannel.invokeMethod<String>(
    _transcodeVideoToMp4Method,
    filePath,
  );
  if (result == null || result.isEmpty) {
    throw Exception('Failed to convert video to MP4.');
  }
  if (defaultTargetPlatform == TargetPlatform.android) {
    final source = File(result);
    final destination = File(
      '$result.faststart-${DateTime.now().microsecondsSinceEpoch}.mp4',
    );
    try {
      await rewriteMp4ForFastStart(source, destination);
      await source.delete();
      return destination.path;
    } catch (_) {
      try {
        await destination.delete();
      } on FileSystemException {
        // Best-effort cleanup; preserve the original platform error.
      }
      rethrow;
    }
  }
  return result;
}

Future<String> _packagePickedVoiceNoteForUpload(String filePath) async {
  final result = await _mediaUploadPlatformChannel.invokeMethod<String>(
    _packageVoiceNoteForUploadMethod,
    filePath,
  );
  if (result == null || result.isEmpty) {
    throw Exception('Failed to prepare voice note for upload.');
  }
  if (defaultTargetPlatform == TargetPlatform.android) {
    final source = File(result);
    final destination = File(
      '$result.faststart-${DateTime.now().microsecondsSinceEpoch}.mp4',
    );
    try {
      await rewriteMp4ForFastStart(source, destination);
      await source.delete();
      return destination.path;
    } catch (_) {
      for (final file in [destination, source]) {
        try {
          if (await file.exists()) await file.delete();
        } on FileSystemException {
          // Best-effort cleanup; preserve the original platform error.
        }
      }
      rethrow;
    }
  }
  return result;
}

Future<Uint8List> _transcodePickedImageToJpeg(Uint8List bytes) async {
  return _invokeRequiredPlatformBytesMethod(
    _transcodeImageToJpegMethod,
    arguments: bytes,
    errorMessage: 'failed to convert image for upload',
  );
}

Future<Uint8List> _sanitizePickedImageBytes(
  Uint8List bytes,
  String mimeType,
) async {
  return _invokeRequiredPlatformBytesMethod(
    _sanitizeImageForUploadMethod,
    arguments: {'bytes': bytes, 'mimeType': mimeType},
    errorMessage: 'failed to sanitize image for upload',
  );
}

Future<Uint8List> _invokeRequiredPlatformBytesMethod(
  String method, {
  Object? arguments,
  required String errorMessage,
}) async {
  final result = await _mediaUploadPlatformChannel.invokeMethod<Uint8List>(
    method,
    arguments,
  );
  if (result == null || result.isEmpty) {
    throw Exception(errorMessage);
  }
  return result;
}
