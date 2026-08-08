import 'dart:io';

import 'package:image_picker/image_picker.dart';

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
