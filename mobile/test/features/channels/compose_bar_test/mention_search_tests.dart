part of '../compose_bar_test.dart';

/// The directory is searched only for a live `@` query that could still
/// name someone (buzz-app `src/bundled/mentions/README.md`, sections 2 and
/// 6; project recommendation 5). Prose typed after a picked chip never
/// leaves the device. A typed, unpicked query searches at every pause, as
/// an empty result says nothing sound about a longer query.
/// The chooser itself follows the field's focus (recommendation 8).
void mentionSearchTests() {
  final bob = 'b' * 64;
  final alice = 'a' * 64;
  final zoe = '9' * 64;
  final members = [
    for (final (key, name) in [(bob, 'Bob'), (alice, 'Alice'), (zoe, 'Zoë')])
      ChannelMember(
        pubkey: key,
        displayName: name,
        role: 'member',
        joinedAt: DateTime(2025),
      ),
  ];
  final quinn = UserProfile(pubkey: 'c' * 64, displayName: 'Quinn');
  final joseMaria = UserProfile(pubkey: 'd' * 64, displayName: 'José María');
  final aliceSmith = UserProfile(pubkey: 'e' * 64, displayName: 'Alice Smith');
  // Found for `bo` too, so only the chip rule keeps the prose after a
  // picked Bob off the relay.
  final bobby = UserProfile(pubkey: 'f' * 64, displayName: 'Bobby');
  String fold(String text) =>
      text.toLowerCase().replaceAll('é', 'e').replaceAll('í', 'i');
  final field = find.byType(TextField);
  final popover = find.byKey(const ValueKey('mention-suggestions-popover'));
  Finder row(String name) =>
      find.descendant(of: popover, matching: find.text(name));

  /// Lets a debounced search for the current query fire.
  Future<void> pause(WidgetTester tester) async {
    await tester.pump();
    await tester.pump(mentionSearchDebounce * 2);
  }

  /// Types [text] one character at a time, pausing after each.
  Future<void> type(WidgetTester tester, String prefix, String text) async {
    for (var i = 1; i <= text.length; i++) {
      await tester.enterText(field, prefix + text.substring(0, i));
      await pause(tester);
    }
  }

  testWidgets(
    'the directory is searched only while a query could still name someone',
    (tester) async {
      final searched = <String>[];
      String? sentText;
      List<List<String>>? sentTags;
      await tester.pumpWidget(
        _buildComposeBar(
          uploadService: _testUploadService(nostr.Keys.generate().nsec),
          currentPubkey: nostr.Keys.generate().public,
          members: members,
          channels: [_makeCurrentChannel()],
          // The relay matches each query word against the start of a name
          // word, folding accents, so a longer query never finds anyone a
          // shorter one did not.
          searchPeople: (query) async {
            searched.add(query);
            final words = fold(query).split(' ').where((w) => w.isNotEmpty);
            return [
              for (final person in [quinn, joseMaria, aliceSmith, bobby])
                if (words.every(
                  (word) => fold(
                    person.displayName!,
                  ).split(' ').any((name) => name.startsWith(word)),
                ))
                  person,
            ];
          },
          onSend: (text, _, {mediaTags = const <List<String>>[]}) async {
            sentText = text;
            sentTags = mediaTags;
          },
        ),
      );
      await _expandComposer(tester);
      final controller = tester.widget<TextField>(field).controller!;

      // A sentence typed after a picked chip is prose, not a name search.
      await tester.enterText(field, 'hi @bo');
      await tester.pumpAndSettle();
      await tester.tap(row('Bob'));
      await tester.pumpAndSettle();
      expect(controller.text, 'hi @Bob ');
      expect(searched, ['bo']);
      searched.clear();
      await type(tester, 'hi @Bob ', 'please review the notes');
      expect(controller.text, 'hi @Bob please review the notes');
      expect(searched, isEmpty);
      expect(popover, findsNothing);

      // Nor is moving the caret back through it.
      for (var caret = controller.text.length - 1; caret >= 0; caret--) {
        controller.selection = TextSelection.collapsed(offset: caret);
        await pause(tester);
      }
      expect(searched, isEmpty);
      expect(popover, findsNothing);

      // A short query that matches nobody here still finds a person
      // outside the channel.
      await tester.enterText(field, '');
      await tester.pump();
      await type(tester, '', '@qu');
      expect(searched.where((q) => q.isNotEmpty), ['q', 'qu']);
      expect(row('Quinn'), findsOneWidget);
      expect(row('not in channel'), findsOneWidget);

      // Closing punctuation after a person the directory found selects them
      // as Space does once the search has finished (recommendation 7), and
      // the send meets the outside-channel prompt as after a tapped pick.
      await type(tester, '@qu', 'inn');
      expect(row('Quinn'), findsOneWidget);
      await tester.enterText(field, '@quinn,');
      await tester.pumpAndSettle();
      expect(controller.text, '@Quinn,');
      expect(
        find.byKey(const ValueKey('composer-person-mention-chip')),
        findsOneWidget,
      );
      expect(popover, findsNothing);
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(find.text('Mention people outside this channel?'), findsOneWidget);
      // Send without inviting: Quinn is referenced, not notified.
      await tester.tap(
        find
            .descendant(
              of: find.byType(AlertDialog),
              matching: find.byType(TextButton),
            )
            .first,
      );
      await tester.pumpAndSettle();
      expect(sentText, '@Quinn,');
      expect(sentTags, contains(orderedEquals(['mention', quinn.pubkey])));
      // The prompt took focus; the composer is a capsule again.
      await _expandComposer(tester);

      // Multi-word names are searched word by word: a second word that
      // continues no local name still goes to the directory, and one that
      // continues a found name keeps that person listed. Names fold accents
      // (section 3, recommendation 6): `jose maria` lists José María.
      searched.clear();
      await tester.enterText(field, '');
      await tester.pump();
      await type(tester, '', '@jose maria');
      expect(searched, contains('jos'));
      expect(searched.last, 'jose maria');
      expect(row('José María'), findsOneWidget);
      searched.clear();
      await tester.enterText(field, '');
      await tester.pump();
      await type(tester, '', '@alice sm');
      expect(searched.last, 'alice sm');
      expect(row('Alice Smith'), findsOneWidget);

      // A search that finds nobody does not stop the search for a longer
      // query (review of #9699). The relay reads `alpha-beta` as three
      // prefix terms and `alpha-beta.com` as one, so the longer query can
      // find a person the shorter one could not. A query the directory
      // cannot match still matches members on the device, folding accents:
      // `@zoe` lists the member Zoë.
      searched.clear();
      await tester.enterText(field, '');
      await tester.pump();
      await type(tester, '', '@zoe');
      expect(searched.where((q) => q.isNotEmpty), ['z', 'zo', 'zoe']);
      expect(row('Zoë'), findsOneWidget);
      searched.clear();
      await tester.enterText(field, '');
      await tester.pump();
      await tester.enterText(field, '@alpha-beta');
      await pause(tester);
      expect(searched, ['alpha-beta']);
      await type(tester, '@alpha-beta', '.com');
      expect(searched.last, 'alpha-beta.com');
      // Prose after a typed, unpicked unknown name searches at each pause.
      await tester.enterText(field, '@alpha-beta.com please');
      await pause(tester);
      expect(searched.last, 'alpha-beta.com please');
    },
  );

  testWidgets('the chooser follows the field\'s focus and keeps its rows', (
    tester,
  ) async {
    final searched = <String>[];
    final focusNode = FocusNode();
    addTearDown(focusNode.dispose);
    String? sentText;
    List<String>? sent;
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _testUploadService(nostr.Keys.generate().nsec),
        currentPubkey: nostr.Keys.generate().public,
        members: members,
        channels: [_makeCurrentChannel()],
        focusNode: focusNode,
        searchPeople: (query) async {
          searched.add(query);
          if (query == 'xq' && searched.where((q) => q == 'xq').length == 1) {
            throw StateError('relay down');
          }
          return fold(bobby.displayName!).startsWith(fold(query))
              ? [bobby]
              : const [];
        },
        onSend: (text, keys, {mediaTags = const <List<String>>[]}) async {
          sentText = text;
          sent = keys;
        },
      ),
    );
    await _expandComposer(tester);
    final controller = tester.widget<TextField>(field).controller!;
    final channelChooser = find.byKey(const ValueKey('channel-suggestions'));

    /// A press and release with a pointer that is not a finger. Flutter
    /// unfocuses a field on a mouse or stylus press outside its tap region,
    /// so the chooser must still be there, with focus kept, at the release.
    Future<void> press(Finder target, ui.PointerDeviceKind kind) async {
      final gesture = await tester.startGesture(
        tester.getCenter(target),
        kind: kind,
      );
      await tester.pumpAndSettle();
      expect(focusNode.hasFocus, isTrue, reason: '$kind press');
      expect(target, findsOneWidget, reason: '$kind press');
      await gesture.up();
      await gesture.removePointer();
      await tester.pumpAndSettle();
    }

    void expectBobThenBobby() {
      expect(row('Bob'), findsOneWidget);
      expect(row('Bobby'), findsOneWidget);
      expect(
        tester.getTopLeft(row('Bob')).dy,
        lessThan(tester.getTopLeft(row('Bobby')).dy),
      );
    }

    // `@bo` lists the member Bob above the directory's Bobby.
    await tester.enterText(field, 'hi @bo');
    await pause(tester);
    expect(searched, ['bo']);
    expectBobThenBobby();

    // The keyboard dismissed, or a tap elsewhere, takes the field's focus:
    // the chooser closes while the draft and its live query stay.
    focusNode.unfocus();
    await tester.pumpAndSettle();
    expect(popover, findsNothing);
    expect(controller.text, 'hi @bo');
    expect(controller.selection.baseOffset, 6);

    // Focus returning shows the same rows again without a new search.
    await tester.tap(find.text('hi @bo'));
    await tester.pumpAndSettle();
    expect(focusNode.hasFocus, isTrue);
    expectBobThenBobby();
    expect(searched, ['bo']);

    // A tap on a row is not a tap away: it inserts the pick and keeps focus.
    await tester.tap(row('Bob'));
    await tester.pumpAndSettle();
    expect(controller.text, 'hi @Bob ');
    expect(focusNode.hasFocus, isTrue);
    expect(popover, findsNothing);
    await tester.tap(find.byIcon(BuzzIcons.arrowUp));
    await tester.pumpAndSettle();
    expect(sentText, 'hi @Bob');
    expect(sent, [bob]);

    // A stylus or mouse press on a row picks it too: the chooser is part
    // of the field's tap region, so the press leaves focus where it is.
    await _expandComposer(tester);
    for (final (kind, text) in [
      (ui.PointerDeviceKind.stylus, 'a'),
      (ui.PointerDeviceKind.mouse, 'b'),
    ]) {
      await tester.enterText(field, '$text @bo');
      await pause(tester);
      await press(row('Bobby'), kind);
      expect(controller.text, '$text @Bobby ', reason: '$kind');
      expect(popover, findsNothing);
    }

    // Retry answers the same press: the failed search runs again.
    await tester.enterText(field, '@xq');
    await pause(tester);
    expect(find.byKey(const ValueKey('mention-search-error')), findsOneWidget);
    expect(searched.where((q) => q == 'xq'), hasLength(1));
    await press(find.text('Retry'), ui.PointerDeviceKind.mouse);
    await pause(tester);
    expect(searched.where((q) => q == 'xq'), hasLength(2));
    expect(find.byKey(const ValueKey('mention-search-error')), findsNothing);
    expect(focusNode.hasFocus, isTrue);

    // The channel chooser follows focus the same way.
    await tester.enterText(field, '#cur');
    await tester.pumpAndSettle();
    expect(channelChooser, findsOneWidget);
    focusNode.unfocus();
    await tester.pumpAndSettle();
    expect(channelChooser, findsNothing);
    await tester.tap(find.text('#cur'));
    await tester.pumpAndSettle();
    expect(channelChooser, findsOneWidget);
    expect(controller.text, '#cur');
  });
}
