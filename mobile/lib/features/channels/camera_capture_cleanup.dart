import 'dart:io';

import 'package:image_picker/image_picker.dart';

var _retainedImageCounter = 0;

Future<void> processCapturedImage(
  XFile image,
  Future<void> Function(XFile image) onCapture,
) async {
  await processTemporaryImages([image], (images) => onCapture(images.single));
}

/// Processes native-owned [images], then removes their temporary files.
Future<void> processTemporaryImages(
  List<XFile> images,
  Future<void> Function(List<XFile> images) process,
) async {
  try {
    await process(images);
  } finally {
    for (final image in images) {
      final path = image.path;
      if (path.isNotEmpty) {
        try {
          await File(path).delete();
        } on FileSystemException {
          // The native picker may already have removed its temporary file.
        }
      }
    }
  }
}

/// Copies native-owned [images] into composer-owned temporary files.
///
/// Native camera and picker integrations delete their source files as soon as
/// their callback returns. Deferred sends must therefore retain their own copy
/// before queueing a preview for later upload.
Future<List<XFile>> retainTemporaryImages(List<XFile> images) async {
  final retained = <XFile>[];
  try {
    for (final image in images) {
      final sourcePath = image.path;
      final extension = _safeFileExtension(sourcePath);
      final destination = File(
        '${Directory.systemTemp.path}${Platform.pathSeparator}'
        'buzz-compose-${DateTime.now().microsecondsSinceEpoch}-'
        '${_retainedImageCounter++}$extension',
      );
      if (sourcePath.isNotEmpty && await File(sourcePath).exists()) {
        await File(sourcePath).copy(destination.path);
      } else {
        await destination.writeAsBytes(await image.readAsBytes(), flush: true);
      }
      retained.add(
        XFile(destination.path, name: image.name, mimeType: image.mimeType),
      );
    }
    return retained;
  } catch (_) {
    for (final image in retained) {
      try {
        await File(image.path).delete();
      } on FileSystemException {
        // A failed copy may not have created every destination.
      }
    }
    rethrow;
  }
}

String _safeFileExtension(String path) {
  final separator = path.lastIndexOf(Platform.pathSeparator);
  final dot = path.lastIndexOf('.');
  if (dot <= separator || dot == path.length - 1) return '.jpg';
  final extension = path.substring(dot);
  return RegExp(r'^\.[A-Za-z0-9]{1,10}$').hasMatch(extension)
      ? extension
      : '.jpg';
}
