part of '../compose_bar_test.dart';

void durableMentionTests() {
  testWidgets(
    'exact draft recipients survive restart and failed-send recovery',
    (tester) async {
      final keys = nostr.Keys.generate();
      final first = 'a' * 64;
      final second = 'b' * 64;
      var renamed = false;
      var fail = true;
      List<String>? sent;
      Widget build({String? thread}) => _buildComposeBar(
        threadHeadId: thread,
        uploadService: _testUploadService(keys.nsec),
        relayConfig: () => _SwitchableRelayConfigNotifier(
          RelayConfig(baseUrl: 'http://localhost:3000', nsec: keys.nsec),
        ),
        channels: [_makeCurrentChannel()],
        members: [
          for (final key in [first, second])
            ChannelMember(
              pubkey: key,
              displayName: renamed ? 'Renamed' : 'Scout',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
        ],
        onSend: (_, mentions, {mediaTags = const []}) async {
          sent = mentions;
          if (fail) throw Exception('relay rejected');
        },
      );
      await tester.pumpWidget(build());
      await _expandComposer(tester);
      await tester.enterText(find.byType(TextField), '@');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Scout').first);
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), '@Scout @');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Scout').last);
      await tester.pumpAndSettle();
      final draft = tester
          .widget<TextField>(find.byType(TextField))
          .controller!
          .text;
      expect(draft, '@Scout @Scout ($second) ');

      // Same mounted composer, different thread and back: old listeners may
      // not erase the original persisted bindings while restoring another key.
      await tester.pumpWidget(build(thread: 'other'));
      await tester.pumpAndSettle();
      await tester.pumpWidget(build());
      await tester.pumpAndSettle();
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        draft,
      );
      await tester.pumpWidget(const SizedBox.shrink());
      renamed = true;
      await tester.pumpWidget(build());
      await tester.pumpAndSettle();
      await tester.tap(find.text(draft.trim()));
      await tester.pumpAndSettle();
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        draft,
      );
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sent, [first, second]);
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        draft,
      );

      // Recovery must persist the bindings before notifying text listeners.
      await tester.pumpWidget(const SizedBox.shrink());
      fail = false;
      sent = null;
      await tester.pumpWidget(build());
      await tester.pumpAndSettle();
      await tester.tap(find.text(draft.trim()));
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sent, [first, second]);
    },
  );
}
