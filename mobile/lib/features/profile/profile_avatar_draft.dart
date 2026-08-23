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
  Future<BlobDescriptor>? _posterUpload;
  Future<BlobDescriptor>? _animationUpload;

  Future<BlobDescriptor> _uploadPoster(MediaUploadService service) {
    final existing = _posterUpload;
    if (existing != null) return existing;
    late final Future<BlobDescriptor> upload;
    upload = service.uploadBytes(poster, mimeType: 'image/png').catchError((
      Object error,
      StackTrace stackTrace,
    ) {
      if (identical(_posterUpload, upload)) _posterUpload = null;
      Error.throwWithStackTrace(error, stackTrace);
    });
    _posterUpload = upload;
    return upload;
  }

  Future<BlobDescriptor> _uploadAnimation(MediaUploadService service) {
    final existing = _animationUpload;
    if (existing != null) return existing;
    late final Future<BlobDescriptor> upload;
    upload = service.uploadBytes(animation, mimeType: 'image/png').catchError((
      Object error,
      StackTrace stackTrace,
    ) {
      if (identical(_animationUpload, upload)) _animationUpload = null;
      Error.throwWithStackTrace(error, stackTrace);
    });
    _animationUpload = upload;
    return upload;
  }

  @override
  Future<String> upload(MediaUploadService service) async {
    final existing = _uploadedUrl;
    if (existing != null) return existing;
    // Cache each content-addressed part independently. If one request fails,
    // retry only that part so a successful counterpart remains attached to
    // this draft instead of becoming an abandoned duplicate.
    final upload = Future.wait(
      [_uploadPoster(service), _uploadAnimation(service)],
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
