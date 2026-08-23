import 'dart:typed_data';

import '../../shared/animated_avatar.dart';
import '../../shared/relay/relay.dart';

sealed class ProfileAvatarDraft {
  const ProfileAvatarDraft();

  Future<String> upload(MediaUploadService service);
}

final class ProfileUrlAvatarDraft extends ProfileAvatarDraft {
  const ProfileUrlAvatarDraft(this.url);

  final String url;

  @override
  Future<String> upload(MediaUploadService service) async => url;
}

final class ProfileImageAvatarDraft extends ProfileAvatarDraft {
  ProfileImageAvatarDraft(this.bytes);

  final Uint8List bytes;
  Future<String>? _uploadedUrl;

  @override
  Future<String> upload(MediaUploadService service) async {
    final existing = _uploadedUrl;
    if (existing != null) return existing;
    final upload = service
        .uploadBytes(bytes, mimeType: 'image/jpeg')
        .then((descriptor) => descriptor.url);
    _uploadedUrl = upload;
    try {
      return await upload;
    } catch (_) {
      if (identical(_uploadedUrl, upload)) _uploadedUrl = null;
      rethrow;
    }
  }
}

final class ProfileAnimatedAvatarDraft extends ProfileAvatarDraft {
  ProfileAnimatedAvatarDraft({required this.animation, required this.poster});

  final Uint8List animation;
  final Uint8List poster;
  Future<String>? _uploadedUrl;

  @override
  Future<String> upload(MediaUploadService service) async {
    final existing = _uploadedUrl;
    if (existing != null) return existing;
    final upload = Future.wait(
      [
        service.uploadBytes(poster, mimeType: 'image/png'),
        service.uploadBytes(animation, mimeType: 'image/png'),
      ],
    ).then((uploads) => buildAnimatedAvatarUrl(uploads[0].url, uploads[1].url));
    _uploadedUrl = upload;
    try {
      return await upload;
    } catch (_) {
      if (identical(_uploadedUrl, upload)) _uploadedUrl = null;
      rethrow;
    }
  }
}
