part of '../compose_bar_test.dart';

void invitationTests() {
  for (final choice in ['reference', 'cancel', 'failure', 'revisit']) {
    testWidgets('agent invitation $choice preserves deliberate audience', (
      tester,
    ) async {
      final signer = nostr.Keys.generate();
      final agent = 'a' * 64;
      final events = <Map<String, dynamic>>[];
      List<String>? recipients;
      List<List<String>> tags = [];
      final service = _testUploadService(signer.nsec);
      Widget build({String? thread}) => _buildComposeBar(
        uploadService: service,
        currentPubkey: signer.public,
        relayAgents: [_testAgent(agent)],
        channels: [_makeCurrentChannel(), _makeSharedMemberChannel()],
        threadHeadId: thread,
        onSend: (_, keys, {mediaTags = const []}) async {
          recipients = keys;
          tags = mediaTags;
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
          rejectAdds: choice == 'failure',
        ),
      );
      await _expandComposer(tester);
      await tester.enterText(find.byType(TextField), '@hel');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Helper Bot'));
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 300));
      expect(find.text('Invite mentioned people or agents?'), findsOneWidget);
      expect(events.where((e) => e['kind'] == 9000), isEmpty);
      expect(recipients, isNull);
      if (choice == 'revisit') {
        await tester.pumpWidget(build(thread: 'other'));
        await tester.pumpWidget(build());
      }
      if (choice == 'cancel') {
        Navigator.of(tester.element(find.byType(AlertDialog))).pop();
      } else {
        await tester.tap(
          find.text(choice == 'reference' ? 'Send without inviting' : 'Invite'),
        );
      }
      await tester.pumpAndSettle();
      await tester.pump(const Duration(milliseconds: 300));
      if (choice == 'reference') {
        expect(recipients, isEmpty);
        expect(tags, [
          ['mention', agent],
        ]);
      } else {
        expect(recipients, isNull);
        if (choice == 'failure') {
          expect(find.textContaining('Message not sent.'), findsOneWidget);
          ScaffoldMessenger.of(
            tester.element(find.byType(ComposeBar)),
          ).removeCurrentSnackBar();
          await tester.pumpAndSettle();
        }
        await tester.tap(find.text('@Helper Bot'));
        await tester.pumpAndSettle();
        expect(
          tester.widget<TextField>(find.byType(TextField)).controller!.text,
          '@Helper Bot ',
        );
      }
      expect(
        events.where((e) => e['kind'] == 9000),
        hasLength(choice == 'failure' ? 1 : 0),
      );
    });
  }
}
