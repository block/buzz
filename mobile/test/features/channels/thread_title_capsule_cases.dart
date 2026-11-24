part of 'channel_detail_page_test.dart';

final _threadNativeBar = find.byWidgetPredicate(
  (widget) =>
      widget is UiKitView && widget.viewType == 'buzz/ios_navigation_bar',
);

Map _threadCapsuleParams(WidgetTester tester) =>
    tester.widget<UiKitView>(_threadNativeBar).creationParams! as Map;

/// The glyph Flutter renders into the capsule's subtitle.
IconData? _threadCapsuleIcon(WidgetTester tester) =>
    tester.widget<IosNavigationBar>(find.byType(IosNavigationBar)).subtitleIcon;

void threadTitleCapsuleTests() {
  for (final dm in [false, true]) {
    testWidgets('iOS thread capsule names its live conversation and goes to '
        'it dm=$dm', (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      const nativeChannel = MethodChannel('buzz/ios_navigation_bar/43');
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        nativeChannel,
        (_) async => null,
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          nativeChannel,
          null,
        ),
      );
      final channel = Channel(
        id: _testChannel.id,
        visibility: 'open',
        description: '',
        createdBy: 'self',
        createdAt: _testChannel.createdAt,
        memberCount: 2,
        isMember: true,
        name: dm ? 'DM' : 'engineering',
        channelType: dm ? 'dm' : 'stream',
        participantPubkeys: dm ? ['self', 'alice'] : [],
        participants: dm ? ['Me', 'Alice'] : [],
      );
      final channels = _FakeChannelsNotifier([channel]);
      final root = _textMsg(
        id: 'capsule-root',
        pubkey: 'alice',
        content: 'Root',
      );
      final timeline = formatTimeline([root]);
      await tester.pumpWidget(
        _buildTestable(
          messages: [root],
          channel: channel,
          channelsNotifier: channels,
          users: const {
            'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
          },
          home: ThreadDetailPage(
            threadHead: timeline.single,
            allMessages: timeline,
            channelId: channel.id,
            currentPubkey: 'self',
            isMember: true,
            isArchived: false,
          ),
        ),
      );
      await tester.pumpAndSettle();
      Map params() => _threadCapsuleParams(tester);
      expect(params()['title'], 'Thread');
      // The Messages list's glyph stands in for a written "#"; a DM has none.
      expect(params()['subtitle'], dm ? 'DM · Alice' : 'engineering');
      expect(_threadCapsuleIcon(tester), dm ? null : BuzzIcons.hash);
      expect(params()['subtitleIcon'], !dm);
      expect(params()['titleDisclosure'], true);
      expect(params()['titleEnabled'], true);
      expect(params()['titleRefreshing'], false);
      // It says where it goes, not the capsule's "Open settings for …".
      expect(
        params()['titleAccessibilityLabel'],
        dm ? 'Go to DM · Alice' : 'Go to engineering',
      );
      // A long name truncates inside the capsule rather than widening it.
      expect(
        params()['titleMaxWidth'],
        conversationTitleMaxWidth(
          tester.view.physicalSize.width / tester.view.devicePixelRatio,
        ),
      );
      channels.setChannels([channel.copyWith(name: 'Renamed conversation')]);
      await tester.pumpAndSettle();
      expect(
        params()['subtitle'],
        dm ? 'DM · Renamed conversation' : 'Renamed conversation',
      );
      expect(params()['titleEnabled'], true);

      // Unresolved, the capsule is the inert one it was before it led up.
      channels.setChannels([]);
      await tester.pumpAndSettle();
      // An empty subtitle keeps the shared capsule while metadata is absent.
      expect(params()['subtitle'], '');
      expect(params()['titleEnabled'], false);
      expect(params()['subtitleIcon'], false);
      expect(params()['titleDisclosure'], false);
      expect(params()['titleRefreshing'], false);
      expect(params()['titleAccessibilityLabel'], isNull);
      expect(params()['titleMaxWidth'], isNull);

      // Tapping the native capsule goes up to the conversation.
      channels.setChannels([channel]);
      await tester.pumpAndSettle();
      tester.widget<UiKitView>(_threadNativeBar).onPlatformViewCreated!(43);
      await tester.pump();
      await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
        nativeChannel.name,
        nativeChannel.codec.encodeMethodCall(
          const MethodCall('action', 'title'),
        ),
        (_) {},
      );
      await tester.pumpAndSettle();
      expect(find.byType(ChannelDetailPage), findsOneWidget);
      expect(find.byType(ThreadDetailPage), findsNothing);
      expect(tester.takeException(), isNull);
      debugDefaultTargetPlatformOverride = null;
    });
  }

  testWidgets('iOS sub-thread capsule shows its conversation while the '
      'parent thread loads, then goes to that thread', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    const nativeChannel = MethodChannel('buzz/ios_navigation_bar/44');
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
      nativeChannel,
      (_) async => null,
    );
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        nativeChannel,
        null,
      ),
    );
    final root = _textMsg(
      id: threadIdRoot,
      pubkey: 'alice',
      content: 'Release plan',
      createdAt: 1000,
    );
    final head = _textMsg(
      id: threadIdParent,
      pubkey: 'bob',
      content: 'Sub-thread head',
      createdAt: 1100,
      extraTags: const [
        ['e', threadIdRoot, '', 'reply'],
      ],
    );
    final session = _ParentLookupSession([root]);
    final timeline = formatTimeline([head]);
    await tester.pumpWidget(
      _buildTestable(
        messages: const [],
        relaySessionNotifier: session,
        threadReplies: {
          threadIdRoot: [head],
        },
        home: ThreadDetailPage(
          threadHead: timeline.single,
          allMessages: timeline,
          channelId: _channelId,
          currentPubkey: 'self',
          isMember: true,
          isArchived: false,
        ),
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 100));
    Map params() => _threadCapsuleParams(tester);

    // Inert and visibly loading, never a tap that does nothing, and
    // already showing the conversation as a top-level thread's capsule does.
    expect(session.requested, [threadIdRoot]);
    expect(params()['subtitle'], 'general');
    expect(_threadCapsuleIcon(tester), BuzzIcons.hash);
    expect(params()['subtitleIcon'], true);
    expect(params()['titleEnabled'], false);
    expect(params()['titleRefreshing'], true);
    expect(params()['titleDisclosure'], false);
    expect(params()['titleAccessibilityLabel'], 'Loading parent thread');

    // Loaded, it looks the same and gains its chevron; only the spoken
    // label tells that it leads to the parent thread.
    session.gate.complete();
    await tester.pumpAndSettle();
    expect(params()['subtitle'], 'general');
    expect(_threadCapsuleIcon(tester), BuzzIcons.hash);
    expect(params()['subtitleIcon'], true);
    expect(params()['titleEnabled'], true);
    expect(params()['titleRefreshing'], false);
    expect(params()['titleDisclosure'], true);
    expect(params()['titleAccessibilityLabel'], 'Go to parent thread');

    // Tapping the native capsule goes up one level: to the parent thread,
    // with this sub-thread's head marked.
    tester.widget<UiKitView>(_threadNativeBar).onPlatformViewCreated!(44);
    await tester.pump();
    await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
      nativeChannel.name,
      nativeChannel.codec.encodeMethodCall(const MethodCall('action', 'title')),
      (_) {},
    );
    await tester.pumpAndSettle();
    await tester.pump(const Duration(seconds: 4));
    expect(_shownThread(tester).threadHead.id, threadIdRoot);
    expect(_shownThread(tester).initialMessageId, threadIdParent);
    expect(tester.takeException(), isNull);
    debugDefaultTargetPlatformOverride = null;
  });
}
