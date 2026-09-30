import 'package:buzz/features/profile/profile_avatar_draft.dart';
import 'package:buzz/shared/animated_avatar.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  test('animated draft retries only the failed upload part', () async {
    final service = _PartiallyFailingUploadService();
    addTearDown(service.dispose);
    final draft = ProfileAnimatedAvatarDraft(
      poster: Uint8List.fromList([1]),
      animation: Uint8List.fromList([2]),
    );

    await expectLater(draft.upload(service), throwsException);
    expect(service.uploadedParts, ['poster', 'animation']);

    final url = await draft.upload(service);
    expect(service.uploadedParts, ['poster', 'animation', 'animation']);
    expect(parseAnimatedAvatarUrl(url)?.posterUrl, 'https://relay/poster.png');
    expect(
      parseAnimatedAvatarUrl(url)?.animationUrl,
      'https://relay/animation.png',
    );

    expect(await draft.upload(service), url);
    expect(service.uploadedParts, ['poster', 'animation', 'animation']);
  });

  test('animated draft reuploads every part for a new community', () async {
    final first = _RecordingUploadService('first');
    final second = _RecordingUploadService('second');
    addTearDown(first.dispose);
    addTearDown(second.dispose);
    final draft = ProfileAnimatedAvatarDraft(
      poster: Uint8List.fromList([1]),
      animation: Uint8List.fromList([2]),
    );

    final firstUrl = await draft.upload(first);
    final secondUrl = await draft.upload(second);

    expect(first.uploadedParts, ['poster', 'animation']);
    expect(second.uploadedParts, ['poster', 'animation']);
    expect(firstUrl, contains('https://first.example/'));
    expect(secondUrl, contains('https://second.example/'));
  });
}

final class _RecordingUploadService extends MediaUploadService {
  _RecordingUploadService(this.community)
    : super(
        baseUrl: 'https://$community.example',
        nsec: null,
        pickGalleryImage: () async => null,
        pickGalleryVideo: () async => null,
      );

  final String community;
  final uploadedParts = <String>[];

  @override
  Future<BlobDescriptor> uploadBytes(
    Uint8List bytes, {
    required String mimeType,
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) async {
    final part = bytes.single == 1 ? 'poster' : 'animation';
    uploadedParts.add(part);
    return BlobDescriptor(
      url: 'https://$community.example/$part.png',
      sha256: '$community-$part',
      size: bytes.length,
      type: mimeType,
      uploaded: 1,
    );
  }
}

final class _PartiallyFailingUploadService extends MediaUploadService {
  _PartiallyFailingUploadService()
    : super(
        baseUrl: 'https://relay.example',
        nsec: null,
        pickGalleryImage: () async => null,
        pickGalleryVideo: () async => null,
      );

  final uploadedParts = <String>[];
  var _failedAnimationOnce = false;

  @override
  Future<BlobDescriptor> uploadBytes(
    Uint8List bytes, {
    required String mimeType,
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) async {
    final part = bytes.single == 1 ? 'poster' : 'animation';
    uploadedParts.add(part);
    if (part == 'animation' && !_failedAnimationOnce) {
      _failedAnimationOnce = true;
      throw Exception('animation upload failed');
    }
    return BlobDescriptor(
      url: 'https://relay/$part.png',
      sha256: '$part-hash',
      size: bytes.length,
      type: mimeType,
      uploaded: 1,
    );
  }
}
