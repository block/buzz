part of '../compose_bar_test.dart';

void exactMentionTests() {
  final first = 'a' * 64;
  final second = 'b' * 64;
  List<ChannelMember> members() => [
    for (final key in [first, second])
      ChannelMember(
        pubkey: key,
        displayName: 'Scout',
        role: 'member',
        joinedAt: DateTime(2025),
      ),
  ];
  testWidgets(
    'same-name picker selections retain exact recipients through removal',
    (tester) async {
      List<String>? sent;
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(nostr.Keys.generate().nsec),
          members: members(),
          channels: [_makeCurrentChannel()],
          onSend: (_, keys, {mediaTags = const []}) async {
            sent = keys;
          },
        ),
      );
      await _expandComposer(tester);
      await tester.enterText(find.byType(TextField), '@');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Scout').first);
      await tester.pumpAndSettle();
      final controller = tester
          .widget<TextField>(find.byType(TextField))
          .controller!;
      expect(controller.text, '@Scout ');
      await tester.enterText(find.byType(TextField), '@Scout @');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Scout').last);
      await tester.pumpAndSettle();
      expect(controller.text, '@Scout @Scout ($second) ');
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sent, [first, second]);
      await tester.enterText(find.byType(TextField), '@');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Scout').first);
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), '@Scout @');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Scout').last);
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), '@Scout ($second) ');
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sent, [second]);
    },
  );

  testWidgets('ambiguous typed names fail visibly without clearing the draft', (
    tester,
  ) async {
    var sent = false;
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _testUploadService(nostr.Keys.generate().nsec),
        members: members(),
        onSend: (_, _, {mediaTags = const []}) async {
          sent = true;
        },
      ),
    );
    await _expandComposer(tester);
    await tester.enterText(find.byType(TextField), '@Scout hello');
    await tester.pumpAndSettle();
    await tester.tap(find.byIcon(LucideIcons.arrowUp));
    await tester.pumpAndSettle();
    expect(sent, isFalse);
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller!.text,
      '@Scout hello',
    );
    expect(find.textContaining('is ambiguous'), findsOneWidget);
  });
}
