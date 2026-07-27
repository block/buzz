import 'dart:io';

import 'package:image_picker/image_picker.dart';

Future<void> processCapturedImage(
  XFile image,
  Future<void> Function(XFile image) onCapture,
) async {
  try {
    await onCapture(image);
  } finally {
    final path = image.path;
    if (path.isNotEmpty) {
      try {
        await File(path).delete();
      } on FileSystemException {
        // The camera plugin may already have removed its temporary file.
      }
    }
  }
}
