part of '../compose_bar_test.dart';

/// Picked mentions are atomic chips bound to their recipient (buzz-app
/// `src/bundled/mentions/README.md`, "Mention rules"; project
/// recommendations 1 and 2).
void mentionChipTests() {
  final bob = 'b' * 64;
  final alice = 'a' * 64;
  final agent = 'f' * 64;
  final members = [
    for (final (key, name) in [(bob, 'Bob'), (alice, 'Alice')])
      ChannelMember(
        pubkey: key,
        displayName: name,
        role: 'member',
        joinedAt: DateTime(2025),
      ),
  ];
  final personChip = find.byKey(const ValueKey('composer-person-mention-chip'));
  final agentChip = find.byKey(const ValueKey('composer-agent-mention-chip'));
  Finder row(String name) => find.descendant(
    of: find.byKey(const ValueKey('mention-suggestions-popover')),
    matching: find.text(name),
  );

  Future<TextEditingController> mount(
    WidgetTester tester, {
    required ComposeBarOnSend onSend,
  }) async {
    final signer = nostr.Keys.generate();
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _testUploadService(signer.nsec),
        currentPubkey: signer.public,
        members: members,
        relayAgents: [_testAgent(agent)],
        channels: [_makeCurrentChannel(), _makeSharedMemberChannel()],
        searchPeople: (_) async => const [],
        onSend: onSend,
      ),
    );
    await _expandComposer(tester);
    return tester.widget<TextField>(find.byType(TextField)).controller!;
  }

  /// The platform reports a backspace: one character before the caret gone.
  void backspace(WidgetTester tester, TextEditingController controller) {
    final caret = controller.selection.baseOffset;
    tester.testTextInput.updateEditingValue(
      TextEditingValue(
        text: controller.text.replaceRange(caret - 1, caret, ''),
        selection: TextSelection.collapsed(offset: caret - 1),
      ),
    );
  }

  Future<void> arrow(
    WidgetTester tester,
    TextEditingController controller,
    int delta,
  ) async {
    controller.selection = TextSelection.collapsed(
      offset: controller.selection.baseOffset + delta,
    );
    await tester.pump();
  }

  testWidgets(
    'picked people and agents are atomic chips that carry the recipient',
    (tester) async {
      final semantics = tester.ensureSemantics();
      List<String>? sent;
      String? sentText;
      final controller = await mount(
        tester,
        onSend: (text, keys, {mediaTags = const []}) async {
          sentText = text;
          sent = keys;
        },
      );

      // A picked person is one chip, read by assistive technology once.
      await tester.enterText(find.byType(TextField), 'hi @bo');
      await tester.pumpAndSettle();
      await tester.tap(row('Bob'));
      await tester.pumpAndSettle();
      expect(controller.text, 'hi @Bob ');
      expect(personChip, findsOneWidget);
      expect(find.bySemanticsLabel('Mention: Bob'), findsOneWidget);

      // Left and Right step over the chip as one unit.
      await arrow(tester, controller, -1);
      expect(controller.selection.baseOffset, 7);
      await arrow(tester, controller, -1);
      expect(controller.selection.baseOffset, 3);
      await arrow(tester, controller, 1);
      expect(controller.selection.baseOffset, 7);
      await arrow(tester, controller, 1);
      expect(controller.selection.baseOffset, 8);

      // Backspace removes the trailing space, then the whole chip: never
      // `@Bo`, which would send with nobody notified.
      backspace(tester, controller);
      await tester.pump();
      expect(controller.text, 'hi @Bob');
      expect(personChip, findsOneWidget);
      backspace(tester, controller);
      await tester.pump();
      expect(controller.text, 'hi ');
      expect(personChip, findsNothing);

      // The same for an agent, including the recorded case where the whole
      // name was typed before the pick: Backspace must not land on the typed
      // `@Helper Bo` and mistake it for Undo.
      await tester.enterText(find.byType(TextField), 'hi @Helper Bo');
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), 'hi @Helper Bot');
      await tester.pumpAndSettle();
      await tester.tap(row('Helper Bot'));
      await tester.pumpAndSettle();
      expect(controller.text, 'hi @Helper Bot ');
      expect(agentChip, findsOneWidget);
      backspace(tester, controller);
      await tester.pump();
      expect(controller.text, 'hi @Helper Bot');
      expect(agentChip, findsOneWidget);
      backspace(tester, controller);
      await tester.pump();
      expect(controller.text, 'hi ');
      expect(agentChip, findsNothing);

      // A deleted chip takes its recipient with it; the text left behind
      // notifies nobody.
      await tester.enterText(find.byType(TextField), 'hi there');
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sentText, 'hi there');
      expect(sent, isEmpty);

      // A name character typed against a chip turns it back into text, so
      // `@Bobby` is nobody and sends with no recipient.
      await tester.enterText(find.byType(TextField), '@bo');
      await tester.pumpAndSettle();
      await tester.tap(row('Bob'));
      await tester.pumpAndSettle();
      expect(personChip, findsOneWidget);
      backspace(tester, controller);
      await tester.pump();
      tester.testTextInput.updateEditingValue(
        const TextEditingValue(
          text: '@Bobby',
          selection: TextSelection.collapsed(offset: 6),
        ),
      );
      await tester.pumpAndSettle();
      expect(personChip, findsNothing);
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sentText, '@Bobby');
      expect(sent, isEmpty);

      // A pick after an opening bracket notifies exactly like one after a
      // space (recommendation 1).
      for (final bracket in ['(', '[', '{']) {
        sent = null;
        await tester.enterText(find.byType(TextField), 'see $bracket@bo');
        await tester.pumpAndSettle();
        await tester.tap(row('Bob'));
        await tester.pumpAndSettle();
        expect(controller.text, 'see $bracket@Bob ');
        expect(personChip, findsOneWidget);
        await tester.tap(find.byIcon(BuzzIcons.arrowUp));
        await tester.pumpAndSettle();
        expect(sentText, 'see $bracket@Bob');
        expect(sent, [bob], reason: 'picked after $bracket');
      }

      // Space-selecting a typed member name after a bracket binds it too.
      sent = null;
      await tester.enterText(find.byType(TextField), 'see (@alice');
      await tester.pump();
      await tester.enterText(find.byType(TextField), 'see (@alice ');
      await tester.pumpAndSettle();
      expect(controller.text, 'see (@Alice ');
      expect(personChip, findsOneWidget);
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sent, [alice]);

      // Typed, unpicked names keep their existing fallback: an exact unique
      // member name notifies, after a bracket as after a space.
      sent = null;
      await tester.enterText(find.byType(TextField), 'ping [@Bob] now');
      await tester.pumpAndSettle();
      expect(personChip, findsNothing);
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sent, [bob]);
      await tester.pump(mentionSearchDebounce * 2);
      semantics.dispose();
    },
  );
}
