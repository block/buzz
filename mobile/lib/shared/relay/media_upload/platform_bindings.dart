part of '../media_upload.dart';

/// Whether saving media needs Android's pre-scoped-storage runtime permission.
Future<bool> requiresLegacyMediaStoragePermission() async {
  if (defaultTargetPlatform != TargetPlatform.android) {
    return false;
  }
  return await _mediaUploadPlatformChannel.invokeMethod<bool>(
        _requiresLegacyMediaStoragePermissionMethod,
      ) ??
      false;
}

final mediaUploadServiceProvider = Provider<MediaUploadService>((ref) {
  final config = ref.watch(relayConfigProvider);
  final picker = ImagePicker();
  final service = MediaUploadService(
    baseUrl: config.baseUrl,
    nsec: config.nsec,
    pickGalleryImage: () => picker.pickImage(
      source: ImageSource.gallery,
      requestFullMetadata: false,
    ),
    pickCameraImage: () => picker.pickImage(
      source: ImageSource.camera,
      requestFullMetadata: false,
    ),
    pickGalleryImages: () => picker.pickMultiImage(requestFullMetadata: false),
    pickGalleryVideo: () => picker.pickVideo(source: ImageSource.gallery),
    pickAttachmentFile: file_selector.openFile,
  );
  ref.onDispose(service.dispose);
  return service;
});
