part of '../compose_bar_test.dart';

/// Picked mentions are atomic chips bound to their recipient (buzz-app
/// `src/bundled/mentions/README.md`, "Mention rules"; project
/// recommendations 1 to 3).
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

      // Typed directly against a chip (the trailing space removed first), a
      // letter or digit continues the name: `@Bobby` and `@Bob2` are nobody
      // and send with no recipient. Punctuation, straight and curly
      // apostrophes, a hyphen or a slash keep the chip and Bob
      // (recommendation 3).
      Future<void> typeAgainstChip(
        String suffix, {
        required Matcher chip,
      }) async {
        await tester.enterText(find.byType(TextField), '@bo');
        await tester.pumpAndSettle();
        await tester.tap(row('Bob'));
        await tester.pumpAndSettle();
        expect(personChip, findsOneWidget);
        backspace(tester, controller);
        await tester.pump();
        tester.testTextInput.updateEditingValue(
          TextEditingValue(
            text: '@Bob$suffix',
            selection: TextSelection.collapsed(offset: 4 + suffix.length),
          ),
        );
        await tester.pumpAndSettle();
        expect(controller.text, '@Bob$suffix');
        expect(personChip, chip, reason: suffix);
        await tester.tap(find.byIcon(BuzzIcons.arrowUp));
        await tester.pumpAndSettle();
        expect(sentText, '@Bob$suffix');
      }

      for (final suffix in ['by', '2']) {
        await typeAgainstChip(suffix, chip: findsNothing);
        expect(sent, isEmpty, reason: suffix);
      }
      for (final suffix in [
        "'s idea",
        '’s idea',
        '-bot',
        '/Alice',
        ', right',
        '.',
        '!',
        '"',
        ')',
      ]) {
        sent = null;
        await typeAgainstChip(suffix, chip: findsOneWidget);
        expect(sent, [bob], reason: suffix);
      }

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

      // Closing punctuation typed straight after an exact, unique name
      // selects it as Space does, and stays after the chip in place of the
      // space (recommendation 7): `@bob,` is a bound Bob followed by `,`.
      for (final mark in [
        ',',
        '.',
        ';',
        ':',
        '!',
        '?',
        ')',
        ']',
        '}',
        "'",
        '’',
        '"',
        '”',
      ]) {
        sent = null;
        await tester.enterText(find.byType(TextField), 'ping @bob');
        await tester.pump();
        await tester.enterText(find.byType(TextField), 'ping @bob$mark');
        await tester.pumpAndSettle();
        expect(controller.text, 'ping @Bob$mark', reason: mark);
        expect(controller.selection.baseOffset, 'ping @Bob$mark'.length);
        expect(personChip, findsOneWidget, reason: mark);
        await tester.tap(find.byIcon(BuzzIcons.arrowUp));
        await tester.pumpAndSettle();
        expect(sentText, 'ping @Bob$mark');
        expect(sent, [bob], reason: mark);
      }

      // An inexact name (`@bo,`) and a character that is not a terminator
      // (`@bob-`) keep the chooser's query: nothing is selected.
      for (final text in ['@bo,', '@bob-']) {
        await tester.enterText(
          find.byType(TextField),
          text.substring(0, text.length - 1),
        );
        await tester.pump();
        await tester.enterText(find.byType(TextField), text);
        await tester.pumpAndSettle();
        expect(controller.text, text);
        expect(personChip, findsNothing, reason: text);
      }

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
