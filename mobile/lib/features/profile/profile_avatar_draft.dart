import 'dart:typed_data';

import '../../shared/animated_avatar.dart';
import '../../shared/relay/relay.dart';

/// A prepared profile-avatar change that is uploaded only when the user saves.
sealed class ProfileAvatarDraft {
  /// Creates a prepared profile-avatar draft.
  const ProfileAvatarDraft();

  /// Returns the avatar URL for this draft using [service] when upload is
  /// required.
  ///
  /// Implementations cache successful uploads for the same service so a
  /// profile-publish retry does not create duplicate media. Failed uploads may
  /// be retried, and changing services starts a new upload for that community.
  Future<String> upload(MediaUploadService service);
}

/// An avatar draft that already has its final URL and needs no media upload.
final class ProfileUrlAvatarDraft extends ProfileAvatarDraft {
  /// Creates a draft backed by [url].
  const ProfileUrlAvatarDraft(this.url);

  /// The URL that will be written to the profile.
  final String url;

  @override
  Future<String> upload(MediaUploadService service) async => url;
}

/// A locally prepared still image awaiting upload on Save.
final class ProfileImageAvatarDraft extends ProfileAvatarDraft {
  /// Creates a still-image draft from JPEG [bytes].
  ProfileImageAvatarDraft(this.bytes);

  /// The prepared JPEG payload.
  final Uint8List bytes;
  MediaUploadService? _uploadService;
  Future<String>? _uploadedUrl;

  @override
  Future<String> upload(MediaUploadService service) async {
    if (!identical(_uploadService, service)) {
      _uploadService = service;
      _uploadedUrl = null;
    }
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

/// A locally prepared animated avatar and its still poster awaiting upload.
final class ProfileAnimatedAvatarDraft extends ProfileAvatarDraft {
  /// Creates an animated draft from PNG [animation] and [poster] payloads.
  ProfileAnimatedAvatarDraft({required this.animation, required this.poster});

  /// The animated PNG payload.
  final Uint8List animation;

  /// The still PNG poster shown when animation is unavailable or disabled.
  final Uint8List poster;
  MediaUploadService? _uploadService;
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
    if (!identical(_uploadService, service)) {
      _uploadService = service;
      _uploadedUrl = null;
      _posterUpload = null;
      _animationUpload = null;
    }
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
