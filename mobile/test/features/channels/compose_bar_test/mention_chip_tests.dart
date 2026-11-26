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
        // Review of #9699 (typed names in links): a typed name the bubble
        // reads as part of a URL, a link or code notifies nobody, and one
        // in the prose beside it still does.
        ('see https://example.org/(@Bob) now', <String>[]),
        ('[docs](https://example.org/(@Bob)) here', <String>[]),
        ('[docs](https://example.org/b "(@Bob)") here', <String>[]),
        ('![chart (@Bob)](https://example.org/c.png)', <String>[]),
        ('<https://example.org/(@Bob)>', <String>[]),
        ('see [@Bob]\n\n[@Bob]: https://example.org/bob', <String>[]),
        ('`` ` (@Bob) ``', <String>[]),
        ('~~~\n(@Bob)\n~~~', <String>[]),
        ('https://example.org/(@Bob) and @Alice', [alice]),
        ('https://example.org/ (@Bob) and `x` @Alice', [bob, alice]),
        ('[docs](https://example.org/b) @Bob', [bob]),
        (
          '[@bob] says (@Alice)\n\n[other]: https://example.org/o',
          [bob, alice],
        ),
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
    // A person outside the channel who shares the member Alice's name.
    final otherAlice = UserProfile(pubkey: 'd' * 64, displayName: 'Alice');
    final signer = nostr.Keys.generate();
    List<String>? sent;
    String? sentText;
    List<List<String>>? sentTags;
    var directoryDown = false;
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
          if (!directoryDown && query.toLowerCase().startsWith('q')) quinn,
          if (query.toLowerCase().startsWith('ali')) otherAlice,
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
    // Code or Code block button wraps stops being a chip and its binding
    // goes dormant, so the composer shows what the send does: nobody is
    // notified, and a person outside the channel raises no prompt. The
    // saved draft keeps the dormant binding.
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
      expect(saved(), [
        ComposeDraftMention(
          start: open.length,
          end: open.length + 4,
          pubkey: bob,
        ),
      ], reason: open);
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
    expect(saved(), [
      ComposeDraftMention(start: 5, end: 9, pubkey: bob),
      ComposeDraftMention(start: 12, end: 16, pubkey: bob),
    ]);
    await send();
    expect(sentText, '`see @Bob ` @Bob');
    expect(sent, [bob]);
    // Review of #9699 (pick lost after a Code round trip): the toolbar
    // cannot take Code off, so the backticks are deleted by hand. The pick
    // is live again as picked: a member is notified, a person outside the
    // channel meets the prompt, and an outsider who shares a member's name
    // is still that outsider, never the member.
    Future<void> deleteBefore(int caret) async {
      controller.selection = TextSelection.collapsed(offset: caret);
      await tester.pump();
      backspace(tester, controller);
      await tester.pump();
    }

    for (final (name, target, key, outside) in [
      ('Bob', row('Bob'), bob, false),
      ('Quinn', row('Quinn'), quinn.pubkey, true),
      (
        'Alice',
        // The member's row comes first; the outsider's is "not in channel".
        find
            .descendant(
              of: find.byKey(const ValueKey('mention-suggestions-popover')),
              matching: find.textContaining('Alice · '),
            )
            .last,
        otherAlice.pubkey,
        true,
      ),
    ]) {
      sent = sentTags = null;
      await pickRow('@${name.toLowerCase()}', target);
      final end = name.length + 1;
      expect(saved(), [
        ComposeDraftMention(start: 0, end: end, pubkey: key),
      ], reason: name);
      await select(0, end);
      await format(BuzzIcons.code);
      expect(controller.text, '`@$name` ', reason: name);
      expect(personChip, findsNothing, reason: name);
      expect(saved(), [
        ComposeDraftMention(start: 1, end: end + 1, pubkey: key),
      ], reason: name);
      await deleteBefore(end + 2);
      await deleteBefore(1);
      expect(controller.text, '@$name ', reason: name);
      expect(personChip, findsOneWidget, reason: name);
      expect(saved(), [
        ComposeDraftMention(start: 0, end: end, pubkey: key),
      ], reason: name);
      await send();
      if (!outside) {
        expect(find.byType(AlertDialog), findsNothing, reason: name);
        expect(sent, [key], reason: name);
        continue;
      }
      expect(
        find.text('Mention people outside this channel?'),
        findsOneWidget,
        reason: name,
      );
      await tester.tap(
        find
            .descendant(
              of: find.byType(AlertDialog),
              matching: find.byType(TextButton),
            )
            .first,
      );
      await tester.pumpAndSettle();
      expect(sent, isNot(contains(alice)), reason: name);
      expect(sentTags, contains(orderedEquals(['mention', key])), reason: name);
      await _expandComposer(tester);
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

    // Review of #9699 (mention inside a web link): a link's label is the
    // link's own text, where the bubble draws no pill. A chip that ends up
    // inside one stops being a chip, as in code: nobody is notified, a
    // person outside the channel raises no prompt, and the label is sent as
    // written. A chip beside the link keeps its recipient.
    for (final name in ['Bob', 'Quinn']) {
      sent = sentTags = null;
      await pickRow('[see @${name.toLowerCase()}', row(name));
      expect(controller.text, '[see @$name ');
      expect(personChip, findsOneWidget, reason: name);
      await tester.enterText(field, '[see @$name ](https://example.org)');
      await tester.pump();
      expect(personChip, findsNothing, reason: name);
      final key = name == 'Bob' ? bob : quinn.pubkey;
      final dormant = [
        ComposeDraftMention(start: 5, end: 6 + name.length, pubkey: key),
      ];
      expect(saved(), dormant, reason: name);
      await send();
      expect(find.byType(AlertDialog), findsNothing, reason: name);
      expect(sentText, '[see @$name ](https://example.org)', reason: name);
      expect(sent, isEmpty, reason: name);
      expect(sentTags, isEmpty, reason: name);

      // Taking the link away again brings the pick back as picked.
      sent = sentTags = null;
      await pickRow('[see @${name.toLowerCase()}', row(name));
      await tester.enterText(field, '[see @$name ](https://example.org)');
      await tester.pump();
      await tester.enterText(field, '[see @$name ');
      await tester.pump();
      expect(personChip, findsOneWidget, reason: name);
      expect(saved(), dormant, reason: name);
      await send();
      if (name == 'Bob') {
        expect(sent, [bob]);
        continue;
      }
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
      expect(sentTags, contains(orderedEquals(['mention', quinn.pubkey])));
      await _expandComposer(tester);
    }
    sent = null;
    await pickRow('[see @Bob](https://example.org) and @bo', row('Bob'));
    expect(controller.text, '[see @Bob](https://example.org) and @Bob ');
    expect(personChip, findsOneWidget);
    expect(saved(), [ComposeDraftMention(start: 36, end: 40, pubkey: bob)]);
    await send();
    expect(sentText, '[see @Bob](https://example.org) and @Bob');
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

    // Review of #9699 (system Undo): the system's own Undo (an iOS shake or
    // gesture) never reaches the composer's history action. The platform
    // reports it to the undo client, which the composer stands in front
    // of, so it is a confirmed Undo and each chip returns with its own key.
    Future<void> systemHistory(String direction) async {
      await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
        SystemChannels.undoManager.name,
        SystemChannels.undoManager.codec.encodeMethodCall(
          MethodCall('UndoManagerClient.handleUndo', [direction]),
        ),
        (_) {},
      );
      await tester.pump(const Duration(seconds: 1));
    }

    // The field's own undo client still finds itself behind the composer's:
    // that is how it reports its undo state to iOS and lets go on blur.
    final fieldUndo = tester.state<UndoHistoryState<TextEditingValue>>(
      find.byType(UndoHistory<TextEditingValue>),
    );
    expect(UndoManager.client, isNot(same(fieldUndo)));
    expect(UndoManager.client == fieldUndo, isTrue);

    both = await pickBothSams();
    await deleteFirstChip();
    await systemHistory('undo');
    expect(controller.text, '@Sam @Sam ');
    expect(personChip, findsNWidgets(2));
    expect(saved(), both);
    await systemHistory('redo');
    expect(controller.text, ' @Sam ');
    expect(saved().single.pubkey, both.last.pubkey);
    await systemHistory('undo');
    expect(saved(), both);
    sent = null;
    await send();
    expect(sent, [both.first.pubkey, both.last.pubkey]);

    // A name character typed against a chip unbinds it. The system Undo
    // brings the pick back: a person outside the channel meets the prompt
    // and is tagged, and one who shares a member's name is never that
    // member. Deleting the character by hand lands on the same text but is
    // no Undo: the name stays typed text, as on main, so Quinn is not
    // notified and `@Alice` is the channel's own Alice, with no prompt.
    final outsideAlice = find
        .descendant(
          of: find.byKey(const ValueKey('mention-suggestions-popover')),
          matching: find.textContaining('Alice · '),
        )
        .last;
    for (final (name, target, key, raw) in [
      ('Quinn', row('Quinn'), quinn.pubkey, false),
      ('Quinn', row('Quinn'), quinn.pubkey, true),
      ('Alice', outsideAlice, otherAlice.pubkey, false),
      ('Alice', outsideAlice, otherAlice.pubkey, true),
    ]) {
      final reason = '$name raw=$raw';
      sent = sentTags = null;
      await tester.enterText(field, '');
      await tester.pump(const Duration(seconds: 1));
      await pickRow('@${name.toLowerCase()}', target);
      await tester.pump(const Duration(seconds: 1));
      final end = name.length + 1;
      final picked = [ComposeDraftMention(start: 0, end: end, pubkey: key)];
      expect(saved(), picked, reason: reason);
      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: '@${name}x ',
          selection: TextSelection.collapsed(offset: end + 1),
        ),
      );
      await tester.pump(const Duration(seconds: 1));
      expect(personChip, findsNothing, reason: reason);
      expect(saved(), isEmpty, reason: reason);
      if (raw) {
        tester.testTextInput.updateEditingValue(
          TextEditingValue(
            text: '@$name ',
            selection: TextSelection.collapsed(offset: end + 1),
          ),
        );
        await tester.pump(const Duration(seconds: 1));
      } else {
        await systemHistory('undo');
      }
      expect(controller.text, '@$name ', reason: reason);
      if (raw) {
        expect(personChip, findsNothing, reason: reason);
        expect(saved(), isEmpty, reason: reason);
        await send();
        expect(find.byType(AlertDialog), findsNothing, reason: reason);
        expect(sent, name == 'Alice' ? [alice] : isEmpty, reason: reason);
        expect(sentTags, isEmpty, reason: reason);
        continue;
      }
      expect(personChip, findsOneWidget, reason: reason);
      expect(saved(), picked, reason: reason);
      await send();
      expect(
        find.text('Mention people outside this channel?'),
        findsOneWidget,
        reason: reason,
      );
      await tester.tap(
        find
            .descendant(
              of: find.byType(AlertDialog),
              matching: find.byType(TextButton),
            )
            .first,
      );
      await tester.pumpAndSettle();
      expect(sent, isNot(contains(alice)), reason: reason);
      expect(
        sentTags,
        contains(orderedEquals(['mention', key])),
        reason: reason,
      );
      await _expandComposer(tester);
    }

    // The space after a chip deleted, so the next word runs into the name,
    // and put back by the system Undo: the chip is Quinn's again.
    await tester.enterText(field, '');
    await tester.pump(const Duration(seconds: 1));
    await pickRow('@quinn', row('Quinn'));
    await tester.enterText(field, '@Quinn hi');
    await tester.pump(const Duration(seconds: 1));
    final beforeHi = [
      ComposeDraftMention(start: 0, end: 6, pubkey: quinn.pubkey),
    ];
    expect(saved(), beforeHi);
    await deleteBefore(7);
    await tester.pump(const Duration(seconds: 1));
    expect(controller.text, '@Quinnhi');
    expect(saved(), isEmpty);
    await systemHistory('undo');
    expect(controller.text, '@Quinn hi');
    expect(saved(), beforeHi);

    // Review of #9699 (an old pick on text nobody picked): only a confirmed
    // Undo or Redo brings a pick back. Text that lands on what the editor
    // once held with a chip, typed or pasted, is typed text.
    Future<void> type(String added) async {
      for (final rune in added.runes) {
        final text = controller.text + String.fromCharCode(rune);
        tester.testTextInput.updateEditingValue(
          TextEditingValue(
            text: text,
            selection: TextSelection.collapsed(offset: text.length),
          ),
        );
        await tester.pump();
      }
      await tester.pump(mentionSearchDebounce * 2);
      await tester.pumpAndSettle();
    }

    Future<void> paste(String pasted) async {
      final text = controller.text + pasted;
      tester.testTextInput.updateEditingValue(
        TextEditingValue(
          text: text,
          selection: TextSelection.collapsed(offset: text.length),
        ),
      );
      await tester.pump(mentionSearchDebounce * 2);
      await tester.pumpAndSettle();
    }

    Future<void> expectTypedText(
      String text, {
      List<String> to = const [],
    }) async {
      expect(controller.text, text, reason: text);
      expect(personChip, findsNothing, reason: text);
      expect(saved(), isEmpty, reason: text);
      sent = sentTags = null;
      await send();
      expect(find.byType(AlertDialog), findsNothing, reason: text);
      expect(sent, to, reason: text);
      expect(sentTags, isEmpty, reason: text);
      expect(controller.text, isEmpty, reason: text);
    }

    // The editor holds `@Quinn `, `@Quinn.` and `@Quinn`, each with the
    // chip, before the chip is deleted.
    await tester.enterText(field, '');
    await tester.pump(const Duration(seconds: 1));
    await pickRow('@quinn', row('Quinn'));
    await deleteBefore(7);
    await type('.');
    expect(controller.text, '@Quinn.');
    expect(saved(), [
      ComposeDraftMention(start: 0, end: 6, pubkey: quinn.pubkey),
    ]);
    await deleteBefore(7);
    await deleteBefore(6);
    expect(controller.text, '');
    // The directory no longer finds Quinn, so the chooser has nobody for
    // the space or the period to select: what follows is the history alone.
    directoryDown = true;
    ProviderScope.containerOf(
      tester.element(find.byType(ComposeBar)),
    ).invalidate(mentionSearchCacheProvider);
    await type('@Quinn');
    expect(personChip, findsNothing);
    expect(saved(), isEmpty);
    await type(' ');
    expect(controller.text, '@Quinn ');
    expect(personChip, findsNothing);
    expect(saved(), isEmpty);
    await deleteBefore(7);
    await type('.');
    await expectTypedText('@Quinn.');
    for (final pasted in ['@Quinn', '@Quinn ']) {
      await paste(pasted);
      await expectTypedText(pasted);
    }
    // Pick, delete the chip, paste its old label, send: the deleted
    // recipient gets no tag and there is no outside-channel prompt.
    directoryDown = false;
    for (final pasted in ['@Quinn', '@Quinn ']) {
      await pickRow('@quinn', row('Quinn'));
      expect(saved(), hasLength(1), reason: pasted);
      await deleteBefore(7);
      await deleteBefore(6);
      expect(controller.text, '', reason: pasted);
      await paste(pasted);
      await expectTypedText(pasted);
    }

    // A sent draft's picks do not come back in the next draft. Alice from
    // outside the channel picked and sent: `@Alice hi` typed afterwards is
    // the channel's own Alice, with no prompt.
    await pickRow('@alice', outsideAlice);
    expect(saved().single.pubkey, otherAlice.pubkey);
    await send();
    await tester.tap(
      find
          .descendant(
            of: find.byType(AlertDialog),
            matching: find.byType(TextButton),
          )
          .first,
    );
    await tester.pumpAndSettle();
    expect(sentTags, contains(orderedEquals(['mention', otherAlice.pubkey])));
    await _expandComposer(tester);
    await type('@Alice hi');
    await expectTypedText('@Alice hi', to: [alice]);
    // One of two Sams picked and sent: `@Sam hi` typed afterwards is refused as
    // ambiguous, not sent to the earlier Sam.
    await pickRow('@sa', samRows.first);
    expect(saved(), hasLength(1));
    sent = null;
    await send();
    expect(sent, hasLength(1));
    await type('@Sam hi');
    expect(personChip, findsNothing);
    expect(saved(), isEmpty);
    sent = null;
    await send();
    expect(find.textContaining('is ambiguous'), findsOneWidget);
    expect(sent, isNull);
    expect(controller.text, '@Sam hi');
    await tester.pump(const Duration(seconds: 5));
  });

  // Review of #9699: the binding history belongs to one draft. Undo in
  // another draft can bring back the earlier draft's text, since the field
  // is the same, but never a pick made there.
  testWidgets('a draft switch leaves the earlier draft\'s picks behind', (
    tester,
  ) async {
    final signer = nostr.Keys.generate();
    Widget build({String? thread}) => _buildComposeBar(
      threadHeadId: thread,
      uploadService: _testUploadService(signer.nsec),
      currentPubkey: signer.public,
      members: members,
      channels: [_makeCurrentChannel()],
      searchPeople: (_) async => const [],
      onSend: (_, _, {mediaTags = const []}) async {},
    );
    await tester.pumpWidget(build());
    await _expandComposer(tester);
    final field = find.byType(TextField);
    final controller = tester.widget<TextField>(field).controller!;
    await tester.enterText(field, '@bo');
    await tester.pumpAndSettle();
    await tester.tap(row('Bob'));
    await tester.pumpAndSettle();
    await tester.pump(const Duration(seconds: 1));
    expect(controller.text, '@Bob ');
    expect(personChip, findsOneWidget);

    await tester.pumpWidget(build(thread: 'other'));
    await tester.pumpAndSettle();
    await _expandComposer(tester);
    await tester.pump(const Duration(seconds: 1));
    expect(controller.text, '');
    await tester.sendKeyDownEvent(LogicalKeyboardKey.controlLeft);
    await tester.sendKeyEvent(LogicalKeyboardKey.keyZ);
    await tester.sendKeyUpEvent(LogicalKeyboardKey.controlLeft);
    await tester.pump(const Duration(seconds: 1));
    expect(controller.text, '@Bob ');
    expect(personChip, findsNothing);
    final drafts = ProviderScope.containerOf(
      tester.element(find.byType(ComposeBar)),
    ).read(composeDraftsProvider.notifier);
    expect(drafts.draftFor('channel-1')?.mentions, [
      ComposeDraftMention(start: 0, end: 4, pubkey: bob),
    ]);
    // Blur releases the undo client, as the field does by itself.
    expect(UndoManager.client, isNotNull);
    FocusManager.instance.primaryFocus?.unfocus();
    await tester.pump();
    expect(UndoManager.client, isNull);
    await tester.pump(const Duration(seconds: 5));
  });

  testWidgets('a # typed after a chip opens channel suggestions', (
    tester,
  ) async {
    List<String>? sent;
    String? sentText;
    final controller = await mount(
      tester,
      onSend: (text, keys, {mediaTags = const []}) async {
        sentText = text;
        sent = keys;
      },
    );
    final channelRow = find.descendant(
      of: find.byKey(const ValueKey('channel-suggestions-popover')),
      matching: find.text('current'),
    );

    for (final (typed, name, chip) in [
      ('@Helper', 'Helper Bot', agentChip),
      ('@bo', 'Bob', personChip),
    ]) {
      await tester.enterText(find.byType(TextField), typed);
      await tester.pumpAndSettle();
      await tester.tap(row(name));
      await tester.pumpAndSettle();
      expect(chip, findsOneWidget);

      // The chip's `@` opens no query, so the `#` is a channel trigger.
      await tester.enterText(find.byType(TextField), '@$name #cur');
      await tester.pumpAndSettle();
      expect(channelRow, findsOneWidget, reason: name);
      await tester.tap(channelRow);
      await tester.pumpAndSettle();
      expect(controller.text, '@$name #current ');
      expect(chip, findsOneWidget);
    }

    await tester.tap(find.byIcon(BuzzIcons.arrowUp));
    await tester.pumpAndSettle();
    expect(sentText?.trim(), '@Bob #current');
    expect(sent, [bob]);
  });
}
