import 'package:camera/camera.dart';
import 'package:flutter/services.dart';

/// Returns the clockwise correction for an Android animated-avatar frame.
///
/// Android image-stream buffers remain sensor-oriented. The correction must
/// account for both the sensor mount and the orientation in which the device
/// is currently held; front-facing frames are mirrored separately after this
/// rotation.
int animatedAvatarFrameRotationDegrees({
  required int sensorOrientation,
  required DeviceOrientation deviceOrientation,
  required CameraLensDirection lensDirection,
}) {
  final deviceOrientationDegrees = switch (deviceOrientation) {
    DeviceOrientation.portraitUp => 0,
    DeviceOrientation.landscapeRight => 90,
    DeviceOrientation.portraitDown => 180,
    DeviceOrientation.landscapeLeft => 270,
  };
  final facingSign = lensDirection == CameraLensDirection.back ? -1 : 1;
  return (sensorOrientation - deviceOrientationDegrees * facingSign + 360) %
      360;
}
