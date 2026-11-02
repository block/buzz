part of '../image_avatar_capture.dart';

class _CameraPreview extends StatelessWidget {
  const _CameraPreview({required this.controller});

  final CameraController controller;

  @override
  Widget build(BuildContext context) {
    final aspectRatio = 1 / controller.value.aspectRatio;
    return FittedBox(
      fit: BoxFit.cover,
      clipBehavior: Clip.hardEdge,
      child: SizedBox(
        width: imageAvatarCameraPreviewSize * aspectRatio,
        height: imageAvatarCameraPreviewSize,
        child: CameraPreview(controller),
      ),
    );
  }
}
