part of '../compose_bar_test.dart';

void sendLifecycleTests() {
  for (final action in [
    'double send',
    'edit',
    'revisit',
    'unmount',
    'failure',
  ]) {
    testWidgets('send preparation fences $action', (tester) async {
      final gate = Completer<List<ChannelMember>>();
      final members = [
        ChannelMember(
          pubkey: 'a' * 64,
          displayName: 'Alice',
          role: 'member',
          joinedAt: DateTime(2025),
        ),
      ];
      var pending = false;
      var sent = 0;
      final service = _testUploadService(nostr.Keys.generate().nsec);
      Widget build({String? thread}) => _buildComposeBar(
        uploadService: service,
        channels: [_makeCurrentChannel()],
        membersLoader: () => pending ? gate.future : Future.value(members),
        threadHeadId: thread,
        onSend: (_, _, {mediaTags = const []}) async {
          sent++;
        },
      );
      await tester.pumpWidget(build());
      await _expandComposer(tester);
      await tester.enterText(find.byType(TextField), '@ali');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Alice'));
      await tester.pumpAndSettle();
      final container = ProviderScope.containerOf(
        tester.element(find.byType(ComposeBar)),
      );
      pending = true;
      container.invalidate(channelMembersProvider('channel-1'));
      await tester.pump();
      // Invoke twice without a frame: an asynchronous latch is too late.
      final button = tester.widget<GestureDetector>(
        find
            .ancestor(
              of: find.byIcon(LucideIcons.arrowUp),
              matching: find.byType(GestureDetector),
            )
            .first,
      );
      button.onTap!();
      if (action == 'double send') button.onTap!();
      await tester.pump();
      if (action == 'edit') {
        await tester.enterText(find.byType(TextField), 'new intent');
      }
      if (action == 'revisit') {
        await tester.pumpWidget(build(thread: 'other-thread'));
        await tester.pumpWidget(build());
      }
      if (action == 'unmount') await tester.pumpWidget(const SizedBox());
      if (action == 'failure') {
        gate.completeError(Exception('membership unavailable'));
      } else {
        gate.complete(members);
      }
      await tester.pumpAndSettle();
      await tester.pump(const Duration(milliseconds: 300));
      expect(sent, action == 'double send' ? 1 : 0);
      if (action == 'edit') {
        expect(
          tester.widget<TextField>(find.byType(TextField)).controller!.text,
          'new intent',
        );
      }
      if (action == 'failure') {
        expect(find.textContaining('membership unavailable'), findsOneWidget);
        expect(
          tester.widget<TextField>(find.byType(TextField)).controller!.text,
          '@Alice ',
        );
      }
    });
  }
  testWidgets('a revisited source cannot finish an old membership batch', (
    tester,
  ) async {
    final signer = nostr.Keys.generate();
    final gate = Completer<void>();
    final events = <Map<String, dynamic>>[];
    var sends = 0;
    final service = _testUploadService(signer.nsec);
    Widget build({String? thread}) => _buildComposeBar(
      uploadService: service,
      currentPubkey: signer.public,
      relayAgents: [
        _testAgent('b' * 64),
        AgentDirectoryEntry(
          pubkey: 'c' * 64,
          displayName: 'Other Bot',
          respondTo: 'anyone',
          channelIds: const ['shared-channel'],
        ),
      ],
      channels: [_makeCurrentChannel(), _makeSharedMemberChannel()],
      threadHeadId: thread,
      onSend: (_, _, {mediaTags = const []}) async {
        sends++;
      },
    );
    await tester.pumpWidget(build());
    final container = ProviderScope.containerOf(
      tester.element(find.byType(ComposeBar)),
    );
    final session = container.read(relaySessionProvider.notifier);
    session.debugAttachSocketForTest(
      _RecordingRelaySocket(
        events,
        session.debugHandleSocketMessageForTest,
        beforeAcknowledged: (event) async {
          if (event['kind'] == 9000) await gate.future;
        },
      ),
    );
    await _expandComposer(tester);
    await tester.enterText(find.byType(TextField), '@hel');
    await tester.pumpAndSettle();
    await tester.tap(find.text('Helper Bot'));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), '@Helper Bot @oth');
    await tester.pumpAndSettle();
    await tester.tap(find.text('Other Bot'));
    await tester.pumpAndSettle();
    await tester.tap(find.byIcon(LucideIcons.arrowUp));
    await tester.pump();
    expect(events.where((e) => e['kind'] == 9000), hasLength(1));
    await tester.pumpWidget(build(thread: 'other'));
    await tester.pumpWidget(build());
    gate.complete();
    await tester.pumpAndSettle();
    await tester.pump(const Duration(milliseconds: 300));
    expect(sends, 0);
    expect(events.where((e) => e['kind'] == 9000), hasLength(1));
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller!.text,
      '@Helper Bot @Other Bot ',
    );
  });
}
