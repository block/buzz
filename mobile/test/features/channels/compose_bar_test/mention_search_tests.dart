part of '../compose_bar_test.dart';

/// The directory is searched only for a live `@` query that could still
/// name someone (buzz-app `src/bundled/mentions/README.md`, sections 2 and
/// 6; project recommendation 5). Prose typed after a picked chip, and a
/// query a finished search has already refuted, never leave the device.
void mentionSearchTests() {
  final bob = 'b' * 64;
  final alice = 'a' * 64;
  final members = [
    for (final (key, name) in [(bob, 'Bob'), (alice, 'Alice')])
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
          onSend: (_, _, {mediaTags = const <List<String>>[]}) async {},
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

      // Multi-word names are searched word by word: a second word that
      // continues no local name still goes to the directory, and one that
      // continues a found name keeps that person listed.
      searched.clear();
      await tester.enterText(field, '');
      await tester.pump();
      await type(tester, '', '@jos mar');
      expect(searched, contains('jos'));
      expect(searched.last, 'jos mar');
      searched.clear();
      await tester.enterText(field, '');
      await tester.pump();
      await type(tester, '', '@alice sm');
      expect(searched.last, 'alice sm');
      expect(row('Alice Smith'), findsOneWidget);

      // Once a search finds nobody, longer queries are refuted: no search,
      // and the chooser closes. The same query searches again when it is
      // typed afresh.
      searched.clear();
      await tester.enterText(field, '');
      await tester.pump();
      await type(tester, '', '@z');
      expect(searched.where((q) => q.isNotEmpty), ['z']);
      searched.clear();
      await type(tester, '@z', 'ed please review');
      expect(searched, isEmpty);
      expect(popover, findsNothing);
      await tester.enterText(field, '@z');
      await pause(tester);
      expect(searched, ['z']);
    },
  );
}
