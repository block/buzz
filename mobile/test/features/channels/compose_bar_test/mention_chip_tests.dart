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

  /// The platform reports a backspace the way iOS does, from the hardware
  /// key and the on-screen delete key alike: the character before the caret
  /// is selected first, then that selection is removed with the caret at
  /// its start.
  void backspace(WidgetTester tester, TextEditingController controller) {
    final text = controller.text;
    final caret = controller.selection.baseOffset;
    tester.testTextInput.updateEditingValue(
      TextEditingValue(
        text: text,
        selection: TextSelection(baseOffset: caret - 1, extentOffset: caret),
      ),
    );
    tester.testTextInput.updateEditingValue(
      TextEditingValue(
        text: text.replaceRange(caret - 1, caret, ''),
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

      // Review of #9699 (typed fallback): a name nobody picked ends where
      // it always has and is never a Markdown link's label, so an address
      // and a pasted link notify nobody; typed emphasis and brackets around
      // the name still notify.
      for (final (text, recipients) in [
        ('mail @Alice@example.org', <String>[]),
        ('[@Alice](https://example.org/a) wrote', <String>[]),
        ("@Alice's idea", <String>[]),
        ('_@Bob_ and (@Alice)', [bob, alice]),
      ]) {
        sent = null;
        await tester.pump(mentionSearchDebounce * 2);
        await tester.enterText(find.byType(TextField), text);
        await tester.pumpAndSettle();
        expect(personChip, findsNothing, reason: text);
        await tester.tap(find.byIcon(BuzzIcons.arrowUp));
        await tester.pumpAndSettle();
        expect(sentText, text);
        expect(sent, recipients, reason: text);
      }
      await tester.pump(mentionSearchDebounce * 2);
      semantics.dispose();
    },
  );
  // Review of #9699: a composer command and Undo know what they keep, so a
  // chip they leave in place keeps its own recipient (formatting, emoji,
  // the at-sign button, Undo and Redo).
  testWidgets('composer commands and Undo keep each chip bound to its own '
      'recipient', (tester) async {
    final samA = '1' * 64;
    final samB = '2' * 64;
    final bobBot = '3' * 64;
    final quinn = UserProfile(pubkey: 'c' * 64, displayName: 'Quinn');
    final signer = nostr.Keys.generate();
    List<String>? sent;
    String? sentText;
    List<List<String>>? sentTags;
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _testUploadService(signer.nsec),
        currentPubkey: signer.public,
        members: [
          ...members,
          for (final key in [samA, samB])
            ChannelMember(
              pubkey: key,
              displayName: 'Sam',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
          ChannelMember(
            pubkey: bobBot,
            displayName: 'Bob-bot',
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        ],
        channels: [_makeCurrentChannel()],
        customEmoji: const [
          CustomEmoji(shortcode: 'meow', url: 'https://example.com/meow.png'),
        ],
        searchPeople: (query) async => [
          if (query.toLowerCase().startsWith('q')) quinn,
        ],
        onSend: (text, keys, {mediaTags = const []}) async {
          sentText = text;
          sent = keys;
          sentTags = mediaTags;
        },
      ),
    );
    await _expandComposer(tester);
    final field = find.byType(TextField);
    final controller = tester.widget<TextField>(field).controller!;
    List<ComposeDraftMention> saved() =>
        ProviderScope.containerOf(tester.element(find.byType(ComposeBar)))
            .read(composeDraftsProvider.notifier)
            .draftFor('channel-1')
            ?.mentions ??
        const [];
    Future<void> pickRow(String text, Finder target) async {
      await tester.enterText(field, text);
      await tester.pump();
      await tester.pump(mentionSearchDebounce * 2);
      await tester.pumpAndSettle();
      await tester.tap(target);
      await tester.pumpAndSettle();
    }

    Future<void> select(int base, int extent) async {
      controller.selection = TextSelection(
        baseOffset: base,
        extentOffset: extent,
      );
      await tester.pump();
    }

    Future<void> format(IconData icon) async {
      await tester.tap(find.byIcon(BuzzIcons.aLargeSmall));
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(icon));
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Close formatting'));
      await tester.pumpAndSettle();
    }

    Future<void> send() async {
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
    }

    // Bold over a person picked from the directory: the chip stays, and the
    // send still meets the outside-channel prompt.
    await pickRow('@quinn', row('Quinn'));
    expect(controller.text, '@Quinn ');
    await select(0, 6);
    await format(BuzzIcons.bold);
    expect(controller.text, '**@Quinn** ');
    expect(personChip, findsOneWidget);
    expect(saved(), [
      ComposeDraftMention(start: 2, end: 8, pubkey: quinn.pubkey),
    ]);
    await send();
    expect(find.text('Mention people outside this channel?'), findsOneWidget);
    await tester.tap(
      find
          .descendant(
            of: find.byType(AlertDialog),
            matching: find.byType(TextButton),
          )
          .first,
    );
    await tester.pumpAndSettle();
    expect(sentText, '**@Quinn**');
    expect(sentTags, contains(orderedEquals(['mention', quinn.pubkey])));
    await _expandComposer(tester);

    // The other wrapping formats keep a member's chip bound too: the saved
    // binding moves inside the delimiters. Italic writes `_`, which closes
    // emphasis after the chip rather than continuing its name.
    for (final (icon, mark) in [
      (BuzzIcons.italic, '_'),
      (BuzzIcons.strikethrough, '~~'),
    ]) {
      sent = null;
      await pickRow('@bo', row('Bob'));
      await select(0, 4);
      await format(icon);
      expect(controller.text, '$mark@Bob$mark ');
      expect(saved(), [
        ComposeDraftMention(
          start: mark.length,
          end: mark.length + 4,
          pubkey: bob,
        ),
      ]);
      await send();
      expect(sent, [bob], reason: mark);
    }

    // Review of #9699 (mentions in code): code is literal text. A chip the
    // Code or Code block button wraps stops being a chip and loses its
    // recipient, so the composer shows what the send does: nobody is
    // notified, and a person outside the channel raises no prompt.
    for (final (icon, open, close) in [
      (BuzzIcons.code, '`', '`'),
      (BuzzIcons.squareCode, '```\n', '\n```'),
    ]) {
      sent = sentTags = null;
      await pickRow('@bo', row('Bob'));
      expect(personChip, findsOneWidget);
      await select(0, 4);
      await format(icon);
      expect(controller.text, '$open@Bob$close ');
      expect(personChip, findsNothing, reason: open);
      expect(saved(), isEmpty, reason: open);
      await send();
      expect(sentText, '$open@Bob$close');
      expect(sent, isEmpty, reason: open);
      expect(sentTags, isEmpty, reason: open);

      sent = sentTags = null;
      await pickRow('@quinn', row('Quinn'));
      await select(0, 6);
      await format(icon);
      expect(controller.text, '$open@Quinn$close ');
      expect(personChip, findsNothing, reason: open);
      await send();
      expect(find.byType(AlertDialog), findsNothing, reason: open);
      expect(sentText, '$open@Quinn$close');
      expect(sent, isEmpty, reason: open);
      expect(sentTags, isEmpty, reason: open);
    }
    // Backticks typed around a chip do the same, and so does a chip inside
    // a longer code span; a chip beside the code keeps its recipient.
    sent = null;
    await pickRow('@bo', row('Bob'));
    await tester.enterText(field, '`see @Bob ');
    await tester.pump();
    expect(personChip, findsOneWidget);
    await tester.enterText(field, '`see @Bob `');
    await tester.pump();
    expect(personChip, findsNothing);
    await pickRow('`see @Bob ` @bo', row('Bob'));
    expect(controller.text, '`see @Bob ` @Bob ');
    expect(personChip, findsOneWidget);
    expect(saved(), [ComposeDraftMention(start: 12, end: 16, pubkey: bob)]);
    await send();
    expect(sent, [bob]);
    // Taking the code away by hand leaves a typed name: a member's notifies
    // by the typed rules, a person outside the channel is not brought back.
    for (final (name, keys) in [
      ('Bob', [bob]),
      ('Quinn', <String>[]),
    ]) {
      sent = null;
      await pickRow('@${name.toLowerCase()}', row(name));
      await select(0, name.length + 1);
      await format(BuzzIcons.code);
      await tester.enterText(field, '@$name ');
      await tester.pump();
      expect(personChip, findsNothing, reason: name);
      await send();
      expect(find.byType(AlertDialog), findsNothing, reason: name);
      expect(sent, keys, reason: name);
    }
    // A typed, unpicked member name inside code notifies nobody either.
    for (final typed in [
      '`@Bob`',
      '`ask @Bob today`',
      '```\n@Bob\n```',
      '**bold `@Bob ok`**',
    ]) {
      sent = null;
      await tester.enterText(field, typed);
      await tester.pump();
      await send();
      expect(sent, isEmpty, reason: typed);
    }
    sent = null;
    await tester.enterText(field, '`code` @Bob');
    await tester.pump();
    await send();
    expect(sent, [bob]);

    // Review of #9699 (chip suffix): `-bot` typed against Bob's chip spells
    // the member Bob-bot, who is picked too. The two read alike in the
    // draft; the sent text qualifies the chip whose suffix made the longer
    // name, so the bubble cannot draw both as Bob-bot.
    sent = null;
    await pickRow('@bo', row('Bob'));
    await tester.enterText(field, '@Bob-bot and ');
    await tester.pump();
    await pickRow('@Bob-bot and @bo', row('Bob-bot'));
    expect(controller.text, '@Bob-bot and @Bob-bot ');
    expect(saved(), [
      ComposeDraftMention(start: 0, end: 4, pubkey: bob),
      ComposeDraftMention(start: 13, end: 21, pubkey: bobBot),
    ]);
    await send();
    expect(sentText, '@Bob ($bob)-bot and @Bob-bot');
    expect(sent, [bob, bobBot]);
    // The same suffix beside a member nobody picked is qualified too: the
    // bubble may know that member's name.
    sent = null;
    await pickRow('@bo', row('Bob'));
    await tester.enterText(field, '@Bob-bot ');
    await tester.pump();
    await send();
    expect(sentText, '@Bob ($bob)-bot');
    expect(sent, [bob]);

    // An emoji and the at-sign button go in at the caret end of a
    // selection; the selected chip is not replaced.
    sent = null;
    await pickRow('@bo', row('Bob'));
    await select(0, 4);
    await tester.tap(find.byIcon(BuzzIcons.smilePlus));
    await tester.pumpAndSettle();
    await tester.tap(find.byIcon(BuzzIcons.sparkles));
    await tester.pump();
    await tester.tap(find.byTooltip(':meow:'));
    await tester.pumpAndSettle();
    expect(controller.text, ':meow:@Bob ');
    expect(personChip, findsOneWidget);
    await send();
    expect(sent, [bob]);
    await pickRow('@bo', row('Bob'));
    await select(5, 0);
    await tester.tap(find.byIcon(BuzzIcons.atSign));
    await tester.pumpAndSettle();
    expect(controller.text, '@Bob @');
    expect(personChip, findsOneWidget);
    expect(saved(), [ComposeDraftMention(start: 0, end: 4, pubkey: bob)]);

    // Two people named Sam, each picked: deleting the first chip and
    // undoing brings it back with its own key, Redo takes it away again.
    final samRows = find.descendant(
      of: find.byKey(const ValueKey('mention-suggestions-popover')),
      matching: find.textContaining('Sam · '),
    );
    Future<List<ComposeDraftMention>> pickBothSams() async {
      await tester.enterText(field, '');
      await tester.pump(const Duration(seconds: 1));
      await pickRow('@sa', samRows.first);
      await pickRow('@Sam @sa', samRows.last);
      expect(controller.text, '@Sam @Sam ');
      expect(personChip, findsNWidgets(2));
      await tester.pump(const Duration(seconds: 1));
      return saved();
    }

    Future<void> deleteFirstChip() async {
      controller.selection = const TextSelection.collapsed(offset: 4);
      await tester.pump();
      tester.testTextInput.updateEditingValue(
        const TextEditingValue(
          text: '@Sa @Sam ',
          selection: TextSelection.collapsed(offset: 3),
        ),
      );
      await tester.pump(const Duration(seconds: 1));
      expect(controller.text, ' @Sam ');
      expect(personChip, findsOneWidget);
    }

    Future<void> history({bool redo = false}) async {
      await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
      if (redo) await tester.sendKeyDownEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyEvent(LogicalKeyboardKey.keyZ);
      if (redo) await tester.sendKeyUpEvent(LogicalKeyboardKey.shiftLeft);
      await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
      await tester.pump(const Duration(seconds: 1));
    }

    var both = await pickBothSams();
    expect({for (final m in both) m.pubkey}, {samA, samB});
    await deleteFirstChip();
    expect(saved().single.pubkey, both.last.pubkey);
    await history();
    expect(controller.text, '@Sam @Sam ');
    expect(personChip, findsNWidgets(2));
    expect(saved(), both);
    await history(redo: true);
    expect(controller.text, ' @Sam ');
    expect(saved().single.pubkey, both.last.pubkey);
    await history();
    expect(saved(), both);
    sent = null;
    await send();
    expect(sent, [both.first.pubkey, both.last.pubkey]);

    // The system's own Undo (an iOS shake or gesture) restores the text
    // without saying so. The text that came back is unbound and shares its
    // name, so the send asks for a pick: it never borrows the other chip.
    both = await pickBothSams();
    await deleteFirstChip();
    await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
      SystemChannels.undoManager.name,
      SystemChannels.undoManager.codec.encodeMethodCall(
        const MethodCall('UndoManagerClient.handleUndo', ['undo']),
      ),
      (_) {},
    );
    await tester.pump(const Duration(seconds: 1));
    expect(controller.text, '@Sam @Sam ');
    sent = null;
    await send();
    expect(sent, isNull);
    expect(find.textContaining('is ambiguous'), findsOneWidget);
    await tester.pump(const Duration(seconds: 5));
  });
}
