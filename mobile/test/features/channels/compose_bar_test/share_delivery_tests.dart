part of '../compose_bar_test.dart';

void shareDeliveryTests() {
  testWidgets('a share joins or replaces the conversation draft', (
    tester,
  ) async {
    final shares = Directory.systemTemp.createTempSync('pending-shares-');
    addTearDown(() => shares.deleteSync(recursive: true));
    final messenger =
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
    messenger.setMockMethodCallHandler(
      const MethodChannel('buzz/app_group'),
      (call) async => shares.path,
    );
    addTearDown(
      () => messenger.setMockMethodCallHandler(
        const MethodChannel('buzz/app_group'),
        null,
      ),
    );
    PendingShare share(String id, List<String> text, List<String> files) {
      final directory = Directory('${shares.path}/$id')..createSync();
      return PendingShare(
        id: id,
        target: null,
        text: text,
        files: [
          for (final name in files)
            (File('${directory.path}/$name')..writeAsBytesSync(_pngBytes)).path,
        ],
      );
    }

    Future<void> deliver(PendingShare pending) async {
      ProviderScope.containerOf(
        tester.element(find.byType(ComposeBar)),
      ).read(composerShareInboxProvider.notifier).deliver('channel-1', pending);
      for (var i = 0; i < 4; i++) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 20)),
        );
        await tester.pump();
      }
    }

    String? draftText() => ProviderScope.containerOf(
      tester.element(find.byType(ComposeBar)),
    ).read(composeDraftsProvider.notifier).textFor('channel-1');

    String? sent;
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _testUploadService(nostr.Keys.generate().nsec),
        onSend:
            (content, mentions, {mediaTags = const <List<String>>[]}) async {
              sent = content;
            },
      ),
    );
    await _expandComposer(tester);
    await tester.enterText(find.byType(TextField), 'My draft');
    await tester.pumpAndSettle();

    await deliver(
      share(
        '11111111-1111-4111-8111-111111111111',
        ['https://example.org/a'],
        ['shot.png'],
      ),
    );
    expect(find.text('Add to your draft?'), findsOneWidget);
    await tester.tap(find.text('Add'));
    await tester.pumpAndSettle();
    expect(draftText(), 'My draft\nhttps://example.org/a');
    expect(find.byTooltip('Remove attachment'), findsOneWidget);
    // The composer owns copies, so the app group's are removed.
    await tester.runAsync(
      () => Future<void>.delayed(const Duration(milliseconds: 20)),
    );
    expect(shares.listSync(), isEmpty);

    await deliver(share('22222222-2222-4222-8222-222222222222', ['Fresh'], []));
    await tester.tap(find.text('Replace'));
    await tester.pumpAndSettle();
    expect(draftText(), 'Fresh');
    expect(find.byTooltip('Remove attachment'), findsNothing);

    if (find.byType(TextField).evaluate().isEmpty) {
      await tester.tap(find.text('Fresh'));
      await tester.pumpAndSettle();
    }
    await tester.tap(find.byIcon(BuzzIcons.arrowUp));
    await tester.pumpAndSettle();
    expect(sent, 'Fresh');

    // Files that can't be copied still let the shared text through.
    const unreadable = '33333333-3333-4333-8333-333333333333';
    Directory('${shares.path}/$unreadable').createSync();
    await deliver(
      PendingShare(
        id: unreadable,
        target: null,
        text: const ['Still here'],
        files: ['${shares.path}/$unreadable/missing.png'],
      ),
    );
    await tester.pumpAndSettle();
    expect(draftText(), 'Still here');
    expect(find.text('Couldn’t add the shared files.'), findsOneWidget);
    expect(find.byTooltip('Remove attachment'), findsNothing);
  });
}
