part of '../compose_bar_test.dart';

class _SlowDraftPhotoLibrary extends _FakePhotoLibrary {
  final result = Completer<List<XFile>>();
  _SlowDraftPhotoLibrary()
    : super([RecentPhoto(id: 'one', thumbnailBytes: _gifBytes)]);
  @override
  Future<List<XFile>> resolveSelectedPhotos(List<RecentPhoto> photos) =>
      result.future;
}

void soloDraftTests() {
  testWidgets('Solo draft stays busy while selected photos resolve', (
    tester,
  ) async {
    final library = _SlowDraftPhotoLibrary();
    var busy = false;
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _testUploadService(nostr.Keys.generate().nsec),
        draftMembers: const [],
        photoLibrary: library,
        onSendingChanged: (value) => busy = value,
        onSend:
            (content, mentions, {mediaTags = const <List<String>>[]}) async {},
      ),
    );
    await _openAttachmentMenu(tester);
    await tester.tap(find.text('Photos'));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const ValueKey('recent-photo-one')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Add 1 photo'));
    await tester.pump();
    expect(busy, isTrue);
    library.result.completeError(Exception('Photo unavailable'));
    await tester.pumpAndSettle();
    expect(busy, isFalse);
    expect(find.text('Couldn’t prepare photos'), findsOneWidget);
  });

  testWidgets(
    'Solo draft freezes editing during first send and restores on failure',
    (tester) async {
      final delivery = Completer<void>();
      var busy = false;
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(nostr.Keys.generate().nsec),
          draftMembers: const [],
          onSendingChanged: (value) => busy = value,
          onSend: (content, mentions, {mediaTags = const <List<String>>[]}) =>
              delivery.future,
        ),
      );
      await _expandComposer(tester);
      await tester.enterText(find.byType(TextField), 'First message');
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pump();
      expect(busy, isTrue);
      expect(tester.widget<TextField>(find.byType(TextField)).readOnly, isTrue);
      delivery.completeError(Exception('Offline'));
      await tester.pumpAndSettle();
      expect(busy, isFalse);
      expect(
        tester.widget<TextField>(find.byType(TextField)).readOnly,
        isFalse,
      );
      expect(find.text('First message'), findsOneWidget);
    },
  );
}
