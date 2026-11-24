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
    final members = _CountedMembers([
      for (var i = 0; i < 7000; i++)
        ChannelMember(
          pubkey: key(i),
          displayName: i == 6999 ? 'Alex Smith 0 30' : name(i),
          role: 'member',
          joinedAt: DateTime(2025),
        ),
    ]);
    final clock = Stopwatch()..start();
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _testUploadService(signer.nsec),
        members: members,
        channels: [_makeCurrentChannel()],
        currentPubkey: signer.public,
        relayConfig: () => _SwitchableRelayConfigNotifier(
          RelayConfig(baseUrl: 'https://relay.example', nsec: signer.nsec),
        ),
        onSend: (text, keys, {mediaTags = const []}) async {
          sentText = text;
          sent = keys;
        },
        overrides: [
          verifiedProfilesProvider.overrideWith(_SettableVerified.new),
        ],
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

    // Review of #9699: every profile that loaded named the whole roster
    // again to label the chip, and later still walked it once. A change to
    // the naming facts that cannot touch the chip's label now reads no
    // member, and a keystroke beside the chip reads none either.
    final container = ProviderScope.containerOf(
      tester.element(find.byType(ComposeBar)),
    );
    Future<void> profile(int i, String name, {int? owner}) async {
      container
          .read(userCacheProvider.notifier)
          .put(
            UserProfile(
              pubkey: key(i),
              displayName: name,
              ownerPubkey: owner == null ? null : key(owner),
            ),
          );
      await tester.pumpAndSettle();
    }

    Future<void> verify(int i, String? name) async {
      final verified =
          container.read(verifiedProfilesProvider.notifier)
              as _SettableVerified;
      verified.set(key(i), name);
      await tester.pumpAndSettle();
    }

    final plain = find.text('@Alex Smith 0');
    final suffixed = find.textContaining('@Alex Smith 0 · ');
    var named = mentionQualifierNamingPasses;
    var walked = mentionQualifierRosterPasses;
    var read = members.reads;
    void expectUntouched() {
      expect(mentionQualifierNamingPasses, named);
      expect(mentionQualifierRosterPasses, walked);
      expect(members.reads, read);
    }

    void expectRelabelled({required bool suffix}) {
      expect(mentionQualifierNamingPasses, greaterThan(named));
      expect(plain, suffix ? findsNothing : findsOneWidget);
      expect(suffixed, suffix ? findsOneWidget : findsNothing);
      named = mentionQualifierNamingPasses;
      walked = mentionQualifierRosterPasses;
      read = members.reads;
    }

    expect(plain, findsOneWidget);
    // Typed before the chip, with the caret left in the prose: a caret
    // after the typed name would open the chooser, which lists members.
    tester.testTextInput.updateEditingValue(
      const TextEditingValue(
        text: 'so @Alex Smith 0 and @Alex Smith 0 30',
        selection: TextSelection.collapsed(offset: 2),
      ),
    );
    await tester.pumpAndSettle();
    expect(plain, findsOneWidget);
    expectUntouched();
    await profile(5, 'Someone Else');
    expectUntouched();
    // A member who takes the chip's name is its namesake: the chip gains
    // its key suffix, and loses it when the name goes.
    await profile(7, 'Alex Smith 0');
    expectRelabelled(suffix: true);
    // Beside a namesake, an unrelated profile still reads no member.
    await profile(11, 'Yet Another');
    expectUntouched();
    expect(suffixed, findsOneWidget);
    await profile(7, 'Zoë Lopez 7');
    expectRelabelled(suffix: false);
    // A verified name outranks the profile's: one that is nobody's
    // namesake reads no member, and the chip's name verified for another
    // member is found and given up like a profile name.
    await verify(13, 'Someone Verified');
    expectUntouched();
    await verify(13, 'Alex Smith 0');
    expectRelabelled(suffix: true);
    await verify(5, 'Someone Else Verified');
    expectUntouched();
    await verify(13, null);
    expectRelabelled(suffix: false);
    // Two agents of one name are told apart by their owners' names, and
    // by key once the owners share a name: renaming the owner of an agent
    // relabels the chip, though the owner's own name meets nobody's.
    await profile(0, 'Alex Smith 0', owner: 20);
    await profile(7, 'Alex Smith 0', owner: 21);
    expect(plain, findsOneWidget);
    named = mentionQualifierNamingPasses;
    walked = mentionQualifierRosterPasses;
    read = members.reads;
    await profile(11, 'Still Another');
    expectUntouched();
    await profile(21, name(20));
    expectRelabelled(suffix: true);
    await profile(21, name(21));
    expectRelabelled(suffix: false);
    await profile(7, 'Zoë Lopez 7');
    await profile(0, 'Alex Smith 0');
    expect(plain, findsOneWidget);

    expect(await send(), lessThan(40));
    expect(sent, [key(0), key(6999)]);

    // No `@`: no name is looked up at all.
    await tester.enterText(find.byType(TextField), 'release notes are up');
    await tester.pumpAndSettle();
    expect(await send(), 0);
    expect(sentText, 'release notes are up');
    expect(sent, isEmpty);

    // No chip: typing and a loading profile read no member at all.
    named = mentionQualifierNamingPasses;
    walked = mentionQualifierRosterPasses;
    read = members.reads;
    await tester.enterText(find.byType(TextField), 'more notes');
    await profile(9, 'Another Person');
    expect(mentionQualifierNamingPasses, named);
    expect(mentionQualifierRosterPasses, walked);
    expect(members.reads, read);

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

/// Verified names a test sets one at a time.
class _SettableVerified extends VerifiedProfilesNotifier {
  @override
  Map<String, VerifiedProfile> build() => const {};

  void set(String pubkey, String? name) => state = {
    for (final entry in state.entries)
      if (entry.key != pubkey) entry.key: entry.value,
    if (name != null)
      pubkey: VerifiedProfile(
        displayName: name,
        username: name,
        issuerLabel: 'Example Corp',
        verifiedAt: 1000,
        checked: true,
        multipleIssuers: false,
      ),
  };
}

/// A roster that counts each member read from it.
class _CountedMembers extends ListBase<ChannelMember> {
  _CountedMembers(this._members);

  final List<ChannelMember> _members;
  int reads = 0;

  @override
  int get length => _members.length;

  @override
  set length(int _) => throw UnsupportedError('fixed roster');

  @override
  ChannelMember operator [](int index) {
    reads++;
    return _members[index];
  }

  @override
  void operator []=(int index, ChannelMember value) =>
      throw UnsupportedError('fixed roster');
}
