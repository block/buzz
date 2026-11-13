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
      // The composer deletes the pending share once its files are copied.
      final pendingDirectory = Directory('${shares.path}/${pending.id}');
      for (var i = 0; i < 100 && pendingDirectory.existsSync(); i++) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 10)),
        );
        await tester.pump();
      }
      expect(pendingDirectory.existsSync(), isFalse);
      await tester.pump();
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

    // A share with nothing left to add leaves the draft alone, unasked.
    const empty = '44444444-4444-4444-8444-444444444444';
    Directory('${shares.path}/$empty').createSync();
    await deliver(
      PendingShare(
        id: empty,
        target: null,
        text: const [],
        files: ['${shares.path}/$empty/missing.png'],
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('Add to your draft?'), findsNothing);
    expect(draftText(), 'Still here');
    expect(find.text('Couldn’t add the shared files.'), findsOneWidget);
  });

  testWidgets('a share to the open conversation joins the composer on screen', (
    tester,
  ) async {
    // A phone window, where Home shows a conversation on the pane's own
    // navigator under the dispatcher's.
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final conversations = ConversationPaneController();
    addTearDown(conversations.dispose);
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
    final community = Community(
      id: 'community-1',
      name: 'First',
      relayUrl: 'wss://first.example',
      addedAt: DateTime(2026),
    );
    final storage = CommunityStorage(secure: FakeSecureStorage());
    await tester.runAsync(() async {
      await storage.save(community);
      await storage.saveActiveId(community.id);
    });
    final channel = Channel(
      id: 'channel-1',
      name: 'general',
      channelType: 'stream',
      visibility: 'open',
      description: '',
      createdBy: 'creator',
      createdAt: DateTime(2026),
      memberCount: 2,
      isMember: true,
    );
    Directory writeShare(String id, List<String> text, String file) {
      final directory = Directory('${shares.path}/$id')..createSync();
      File('${directory.path}/$file').writeAsBytesSync(_pngBytes);
      File('${directory.path}/share.json').writeAsStringSync(
        jsonEncode({
          'version': 1,
          'target': shareConversationIdentifier(
            communityId: community.id,
            channelId: channel.id,
          ),
          'text': text,
          'files': [file],
        }),
      );
      return directory;
    }

    Future<void> waitForDeletion(Directory directory) async {
      for (var i = 0; i < 100 && directory.existsSync(); i++) {
        await tester.runAsync(
          () => Future<void>.delayed(const Duration(milliseconds: 10)),
        );
        await tester.pump();
      }
      expect(directory.existsSync(), isFalse);
      await tester.pump();
    }

    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _testUploadService(nostr.Keys.generate().nsec),
        onSend:
            (content, mentions, {mediaTags = const <List<String>>[]}) async {},
        channels: [channel],
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          communitySnapshotWriterProvider.overrideWithValue((_) async {}),
          relaySessionProvider.overrideWith(_ConnectedShareSession.new),
          pendingDeepLinkProvider.overrideWith(_NoLaunchLink.new),
        ],
        wrapPage: (page) {
          conversations.open(
            'channel-1',
            (_) => _OpenChannelPage(child: page),
            animate: false,
          );
          return DeepLinkDispatcher(
            destinationBuilder: (channel, link) => const Text('second page'),
            child: ConversationPaneScope(
              controller: conversations,
              child: AdaptiveConversationPane(
                controller: conversations,
                list: const SizedBox.shrink(),
              ),
            ),
          );
        },
      ),
    );
    await _expandComposer(tester);
    await tester.enterText(find.byType(TextField), 'My draft');
    await tester.pumpAndSettle();
    final container = ProviderScope.containerOf(
      tester.element(find.byType(ComposeBar)),
    );

    // The conversation already has a photo when the next share arrives.
    const first = '55555555-5555-4555-8555-555555555555';
    final firstShare = writeShare(first, const [], 'first.png');
    container
        .read(pendingDeepLinkProvider.notifier)
        .open(Uri.parse('buzz://share?id=$first'));
    await waitForDeletion(firstShare);
    await tester.tap(find.text('Add'));
    await tester.pumpAndSettle();
    expect(find.byTooltip('Remove attachment'), findsOneWidget);

    // A thread inside the pane and a page over Home cover the conversation;
    // the share returns to it the way Back would.
    final composerContext = tester.element(find.byType(ComposeBar));
    unawaited(
      Navigator.of(composerContext).push(
        MaterialPageRoute<void>(builder: (_) => const Text('thread page')),
      ),
    );
    unawaited(
      Navigator.of(composerContext, rootNavigator: true).push(
        MaterialPageRoute<void>(builder: (_) => const Text('settings page')),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.text('settings page'), findsOneWidget);

    const second = '66666666-6666-4666-8666-666666666666';
    final secondShare = writeShare(second, const ['Look'], 'second.png');
    container
        .read(pendingDeepLinkProvider.notifier)
        .open(Uri.parse('buzz://share?id=$second'));
    await waitForDeletion(secondShare);
    await tester.tap(find.text('Add'));
    await tester.pumpAndSettle();

    // One composer owns the draft: both photos and the joined text are on
    // screen, and no second page for the conversation was opened.
    expect(find.text('second page'), findsNothing);
    expect(find.text('thread page'), findsNothing);
    expect(find.text('settings page'), findsNothing);
    expect(find.byType(ComposeBar), findsOneWidget);
    expect(find.byTooltip('Remove attachment'), findsNWidgets(2));
    expect(
      container.read(composeDraftsProvider.notifier).textFor('channel-1'),
      'My draft\nLook',
    );
  });
}

class _OpenChannelPage extends HookConsumerWidget {
  const _OpenChannelPage({required this.child});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useOpenChannelRoute(ref, 'channel-1');
    return child;
  }
}

class _ConnectedShareSession extends RelaySessionNotifier {
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
}

/// Pending links without the platform's launch link stream.
class _NoLaunchLink extends PendingDeepLinkNotifier {
  @override
  BuzzDeepLink? build() => null;
}
