part of '../compose_bar_test.dart';

/// Review of #9699: the send path compiled one pattern per member name, so a
/// channel of about 7,000 members froze a plain send for about nine seconds.
void largeRosterTests() {
  const firstNames = ['Alex', 'Sam', 'José', 'Mary-Jane', 'Zoë', 'Kenny'];
  const lastNames = ['Smith', 'Lopez', "O'Brien", 'Nakamura', 'van Dijk'];
  String key(int i) => i.toRadixString(16).padLeft(64, '0');
  String name(int i) =>
      '${firstNames[i % firstNames.length]} '
      '${lastNames[i % lastNames.length]} $i';

  testWidgets('a legacy draft, a plain message and several mentions send '
      'without scanning a 7,000-member roster', (tester) async {
    final signer = nostr.Keys.generate();
    final prefsKey = 'compose_drafts_v1:https://relay.example:${signer.public}';
    await _testPrefs.setString(
      prefsKey,
      jsonEncode([
        {
          'key': 'channel-1',
          'channel_id': 'channel-1',
          // Alex Smith 0 is picked; the longer member name is that member's.
          'text': '@Alex Smith 0 and @Alex Smith 0 30',
          'mention_keys': {'Alex Smith 0': key(0)},
          'updated_at': 1700000000,
        },
      ]),
    );
    String? sentText;
    List<String>? sent;
    final clock = Stopwatch()..start();
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _testUploadService(signer.nsec),
        members: [
          for (var i = 0; i < 7000; i++)
            ChannelMember(
              pubkey: key(i),
              displayName: i == 6999 ? 'Alex Smith 0 30' : name(i),
              role: 'member',
              joinedAt: DateTime(2025),
            ),
        ],
        channels: [_makeCurrentChannel()],
        currentPubkey: signer.public,
        relayConfig: () => _SwitchableRelayConfigNotifier(
          RelayConfig(baseUrl: 'https://relay.example', nsec: signer.nsec),
        ),
        onSend: (text, keys, {mediaTags = const []}) async {
          sentText = text;
          sent = keys;
        },
      ),
    );
    await tester.tap(find.textContaining('@Alex Smith 0'));
    await tester.pumpAndSettle();

    Future<int> send() async {
      final before = mentionScanLookups;
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
      return mentionScanLookups - before;
    }

    // Opening the legacy draft settled it against the roster: the picked
    // name is a chip, the longer member name is typed text.
    final saved = jsonDecode(_testPrefs.getString(prefsKey)!) as List;
    expect(saved.single['mentions'], [
      {'start': 0, 'end': 13, 'pubkey': key(0)},
    ]);
    expect(await send(), lessThan(40));
    expect(sent, [key(0), key(6999)]);

    // No `@`: no name is looked up at all.
    await tester.enterText(find.byType(TextField), 'release notes are up');
    await tester.pumpAndSettle();
    expect(await send(), 0);
    expect(sentText, 'release notes are up');
    expect(sent, isEmpty);

    // Typed names in any case, after a bracket and inside italics; a name
    // nobody has and an address notify nobody.
    await tester.enterText(
      find.byType(TextField),
      "hi @sam lopez 1, (@JOSÉ O'BRIEN 2) and _@Kenny van Dijk 6599_ "
      'not @Nobody Here or mail@Alex Smith 0',
    );
    await tester.pumpAndSettle();
    // Work follows the `@` signs and the distinct name lengths, never the
    // 7,000 names.
    expect(await send(), lessThan(40));
    expect(sent, [key(1), key(2), key(6599)]);

    // Generous: the journey takes about two seconds, and each of its three
    // sends alone took about ten with a pattern per member.
    expect(clock.elapsed, lessThan(const Duration(seconds: 15)));
  });
}
