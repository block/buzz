import 'dart:io';

import 'package:flutter_test/flutter_test.dart';
import 'package:image_picker/image_picker.dart';
import 'package:buzz/features/channels/camera_capture_cleanup.dart';

void main() {
  test('deletes the inline camera file after upload succeeds', () async {
    final file = File(
      '${Directory.systemTemp.path}/buzz-camera-${DateTime.now().microsecondsSinceEpoch}.jpg',
    );
    await file.writeAsBytes([1, 2, 3]);

    await processCapturedImage(XFile(file.path), (_) async {});

    expect(await file.exists(), isFalse);
  });

  test('deletes the inline camera file when upload fails', () async {
    final file = File(
      '${Directory.systemTemp.path}/buzz-camera-${DateTime.now().microsecondsSinceEpoch}.jpg',
    );
    await file.writeAsBytes([1, 2, 3]);

    await expectLater(
      processCapturedImage(
        XFile(file.path),
        (_) async => throw Exception('upload failed'),
      ),
      throwsException,
    );

    expect(await file.exists(), isFalse);
  });

  test('deletes every native picker file after processing', () async {
    final suffix = DateTime.now().microsecondsSinceEpoch;
    final files = [
      File('${Directory.systemTemp.path}/buzz-photo-$suffix-1.jpg'),
      File('${Directory.systemTemp.path}/buzz-photo-$suffix-2.jpg'),
    ];
    for (final file in files) {
      await file.writeAsBytes([1, 2, 3]);
    }

    await processTemporaryImages([
      for (final file in files) XFile(file.path),
    ], (_) async {});

    for (final file in files) {
      expect(await file.exists(), isFalse);
    }
  });
}
