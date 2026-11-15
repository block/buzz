part of '../compose_bar_test.dart';

class _MixedGalleryService extends _FakeVideoUploadService {
  _MixedGalleryService(super.video);
  final uploaded = <String>[];
  @override
  Future<Uint8List?> prepareVideoPoster(String path) async => _gifBytes;
  @override
  Future<List<XFile>> pickGalleryImages() async => [
    XFile.fromData(_pngBytes, name: 'photo.png'),
    video,
  ];
  @override
  Future<BlobDescriptor> uploadImage(
    XFile image, {
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) async {
    uploaded.add('image');
    return const BlobDescriptor(
      url: 'https://relay.example/photo.png',
      sha256: 'photo',
      size: 10,
      type: 'image/png',
      uploaded: 1,
    );
  }

  @override
  Future<BlobDescriptor> uploadVideo(
    XFile video, {
    ValueChanged<double>? onProgress,
    UploadCancellationToken? cancellationToken,
  }) async {
    uploaded.add('video');
    return super.uploadVideo(
      video,
      onProgress: onProgress,
      cancellationToken: cancellationToken,
    );
  }
}

void _mixedGalleryTests() {
  for (final filename in [
    'camera.MOV',
    'clip.mp4',
    'clip.m4v',
    'clip.3gpp',
    'clip.3g2',
  ]) {
    testWidgets('Photos queues $filename as a video alongside photos', (
      tester,
    ) async {
      // XFile.fromData ignores its name on IO platforms; picker results are paths.
      final video = XFile('/fixtures/$filename');
      final service = _MixedGalleryService(video);
      String? sent;
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: service,
          onSend:
              (content, mentions, {mediaTags = const <List<String>>[]}) async {
                sent = content;
              },
        ),
      );
      await _openSystemPhotoPicker(tester);
      await tester.pumpAndSettle();
      expect(find.byTooltip('Remove attachment'), findsNWidgets(2));
      expect(service.uploaded, isEmpty);
      expect(
        find.byWidgetPredicate(
          (widget) =>
              widget is Image &&
              widget.image is MemoryImage &&
              identical((widget.image as MemoryImage).bytes, _gifBytes),
        ),
        findsOneWidget,
      );
      await _expandComposer(tester);
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(service.uploaded, ['image', 'video']);
      expect(service.uploadedVideo, same(video));
      expect(sent, contains('![video]'));
    });
  }
}
