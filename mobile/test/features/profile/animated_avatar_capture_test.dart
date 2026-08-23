import 'package:buzz/features/profile/animated_avatar_orientation.dart';
import 'package:camera/camera.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  group('animatedAvatarFrameRotationDegrees', () {
    test('compensates front camera frames for every device orientation', () {
      const expected = {
        DeviceOrientation.portraitUp: 270,
        DeviceOrientation.landscapeRight: 180,
        DeviceOrientation.portraitDown: 90,
        DeviceOrientation.landscapeLeft: 0,
      };

      for (final entry in expected.entries) {
        expect(
          animatedAvatarFrameRotationDegrees(
            sensorOrientation: 270,
            deviceOrientation: entry.key,
            lensDirection: CameraLensDirection.front,
          ),
          entry.value,
        );
      }
    });

    test('compensates back camera frames for every device orientation', () {
      const expected = {
        DeviceOrientation.portraitUp: 90,
        DeviceOrientation.landscapeRight: 180,
        DeviceOrientation.portraitDown: 270,
        DeviceOrientation.landscapeLeft: 0,
      };

      for (final entry in expected.entries) {
        expect(
          animatedAvatarFrameRotationDegrees(
            sensorOrientation: 90,
            deviceOrientation: entry.key,
            lensDirection: CameraLensDirection.back,
          ),
          entry.value,
        );
      }
    });
  });
}
