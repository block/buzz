/// Returns the portrait-up correction for an Android animated-avatar frame.
///
/// Android image-stream buffers remain sensor-oriented, so the fixed hardware
/// sensor mount still needs correction even though capture is portrait-locked.
int animatedAvatarPortraitFrameRotationDegrees({
  required int sensorOrientation,
}) {
  return (sensorOrientation % 360 + 360) % 360;
}
