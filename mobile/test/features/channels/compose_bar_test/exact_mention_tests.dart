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
  Future<void> mountComposer(
    WidgetTester tester,
    List<ChannelMember> roster,
    ComposeBarOnSend onSend, {
    nostr.Keys? signer,
    List<OwnedAgentDirectoryEntry>? ownedAgents,
    bool ownedAgentsUnknown = false,
  }) async {
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _testUploadService(
          (signer ?? nostr.Keys.generate()).nsec,
        ),
        members: roster,
        channels: [_makeCurrentChannel()],
        currentPubkey: signer?.public,
        relayConfig: signer == null
            ? null
            : () => _SwitchableRelayConfigNotifier(
                RelayConfig(
                  baseUrl: 'https://relay.example',
                  nsec: signer.nsec,
                ),
              ),
        onSend: onSend,
        ownedAgents: ownedAgents,
        ownedAgentsUnknown: ownedAgentsUnknown,
      ),
    );
    if (find.byType(TextField).evaluate().isEmpty &&
        find.textContaining('@Scout').evaluate().isNotEmpty) {
      // A restored draft displays its text, not the empty composer hint.
      await tester.tap(find.textContaining('@Scout'));
      await tester.pumpAndSettle();
    } else {
      await _expandComposer(tester);
    }
  }

  // Same-name members get contextual picker labels (`Scout · <npub suffix>`);
  // a picked namesake's chip shows the same label while the draft text stays
  // the plain name, and each chip is bound to its own key by position.
  Finder scoutRows() => find.descendant(
    of: find.byKey(const ValueKey('mention-suggestions-popover')),
    matching: find.textContaining('Scout'),
  );

  Future<void> pick(
    WidgetTester tester,
    String text, {
    bool last = false,
  }) async {
    await tester.enterText(find.byType(TextField), text);
    await tester.pumpAndSettle();
    final rows = scoutRows();
    await tester.tap(last ? rows.last : rows.first);
    await tester.pumpAndSettle();
  }

  for (final scenario in [
    'rename',
    'map edited',
    'map reset',
    'map plain',
    'map reselected',
    'removed',
    'legacy',
    'legacy map',
    'malformed',
    'tainted',
    'non-member human',
    'non-member denied agent',
    'non-member unlisted own agent',
    'non-member listed own agent',
    'non-member own agent, list unknown',
  ]) {
    testWidgets('persisted selection restore to signed send: $scenario', (
      tester,
    ) async {
      final signer = nostr.Keys.generate();
      final events = <Map<String, dynamic>>[];
      late SendMessage sendMessage;
      Future<void> mount(List<ChannelMember> roster) async {
        await mountComposer(
          tester,
          roster,
          (text, keys, {mediaTags = const []}) => sendMessage(
            channelId: 'channel-1',
            content: text,
            mentionPubkeys: keys,
            mediaTags: mediaTags,
          ),
          signer: signer,
          // The Agents page lists the owned agent only in its own scenario.
          ownedAgents: switch (scenario) {
            'non-member unlisted own agent' => const [],
            'non-member listed own agent' => [
              OwnedAgentDirectoryEntry(pubkey: first),
            ],
            _ => null,
          },
          // Still loading (or offline): nothing is known to be unlisted.
          ownedAgentsUnknown: scenario == 'non-member own agent, list unknown',
        );
        final container = ProviderScope.containerOf(
          tester.element(find.byType(ComposeBar)),
        );
        final session = container.read(relaySessionProvider.notifier);
        session.debugAttachSocketForTest(
          _RecordingRelaySocket(
            events,
            session.debugHandleSocketMessageForTest,
          ),
        );
        sendMessage = SendMessage(
          signedEventRelay: SignedEventRelay(
            session: session,
            nsec: signer.nsec,
          ),
          fetchMembers: (_) async => roster,
          readUserCache: () => const {},
          addLocalMessage: (_, _) {},
          completeLocalMessage: (_, _) {},
          removeLocalMessage: (_, _) {},
        );
      }

      await mount(members());
      await pick(tester, '@');
      final prefsKey =
          'compose_drafts_v1:https://relay.example:${signer.public}';
      final saved = jsonDecode(_testPrefs.getString(prefsKey)!) as List;
      expect(saved.single['text'], '@Scout ');
      expect(saved.single['mentions'], [
        {'start': 0, 'end': 6, 'pubkey': first},
      ]);
      // Dispose the actual provider state, not just the text controller.
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
      if (scenario.startsWith('map ')) saved.single['mentions'] = null;
      if (scenario.startsWith('legacy')) saved.single.remove('mentions');
      if (scenario == 'legacy map') {
        saved.single['mention_keys'] = {'Scout': first};
      }
      if (scenario == 'malformed') {
        saved.single['mentions'] = [
          {'start': 0, 'end': 6, 'pubkey': 'not-a-key'},
        ];
      }
      if (scenario == 'tainted') {
        saved.single['mentions'] = {
          'Scout': {'pubkey': second, 'is_agent': true},
        };
      }
      await _testPrefs.setString(prefsKey, jsonEncode(saved));
      await mount([
        if (![
          'removed',
          'non-member human',
          'non-member denied agent',
          'non-member unlisted own agent',
          'non-member listed own agent',
          'non-member own agent, list unknown',
        ].contains(scenario))
          ChannelMember(
            pubkey: first,
            displayName: 'Renamed',
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        members().last,
      ]);
      if (scenario.startsWith('non-member')) {
        ProviderScope.containerOf(tester.element(find.byType(ComposeBar)))
            .read(userCacheProvider.notifier)
            .put(
              UserProfile(
                pubkey: first,
                displayName: 'Renamed',
                // The viewer's own agent. An earlier Buzz's agent is not on
                // the Agents page; a listed one is.
                ownerPubkey: switch (scenario) {
                  'non-member denied agent' => second,
                  'non-member unlisted own agent' ||
                  'non-member listed own agent' ||
                  'non-member own agent, list unknown' => signer.public,
                  _ => null,
                },
              ),
            );
        await tester.pumpAndSettle();
      }
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        '@Scout ',
      );
      var expectedText = '@Scout ';
      if (scenario.startsWith('map ')) {
        // An unreadable record stays unreadable while any `@` remains, and
        // is persisted that way rather than as plain text.
        expectedText = '@Scout hello';
        await tester.enterText(find.byType(TextField), expectedText);
        await tester.pumpAndSettle();
        final persisted = jsonDecode(_testPrefs.getString(prefsKey)!).single;
        expect(persisted.containsKey('mentions'), isTrue);
        expect(persisted['mentions'], isNull);
        await tester.pumpWidget(const SizedBox.shrink());
        await tester.pumpAndSettle();
        await mount(members());
        if (scenario != 'map edited') {
          expectedText = scenario == 'map plain' ? 'hello' : '';
          await tester.enterText(find.byType(TextField), expectedText);
          await tester.pumpAndSettle();
          if (scenario != 'map plain') {
            await pick(tester, '@', last: scenario == 'map reselected');
            expectedText = '@Scout ';
          }
        }
      }
      // Admitted people and agents outside the channel are asked about.
      final outsider = [
        'non-member human',
        'non-member listed own agent',
        'non-member own agent, list unknown',
      ].contains(scenario);
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
      if (outsider) {
        expect(
          find.textContaining('Renamed is not in this channel'),
          findsOneWidget,
        );
        await tester.tap(find.text('Do nothing'));
        await tester.pumpAndSettle();
      }
      final messages = events.where(
        (e) => e['kind'] == EventKind.streamMessage,
      );
      if ([
        'rename',
        'legacy',
        'legacy map',
        'non-member human',
        'non-member listed own agent',
        'non-member own agent, list unknown',
        'map reset',
        'map plain',
        'map reselected',
      ].contains(scenario)) {
        final event = messages.single;
        expect(event['content'], expectedText.trim());
        expect((event['tags'] as List).where((t) => t[0] == 'p').toList(), [
          if (!outsider && scenario != 'map plain')
            [
              'p',
              // Without any binding, the typed `@Scout` falls back to the
              // one member still called Scout.
              ['legacy', 'map reselected'].contains(scenario) ? second : first,
            ],
        ]);
        if (outsider) {
          expect(
            (event['tags'] as List).where((t) => t[0] == 'mention').toList(),
            [
              ['mention', first],
            ],
          );
          expect(events.where((e) => e['kind'] == 9000), isEmpty);
        }
        expect(event['pubkey'], signer.public);
        expect(event['sig'], matches(RegExp(r'^[0-9a-f]{128}$')));
        expect(nostr.Event.fromMap(event).id, event['id']);
      } else {
        expect(messages, isEmpty);
        expect(events.where((e) => e['kind'] == 9000), isEmpty);
        expect(find.textContaining('Saved mention'), findsOneWidget);
        expect(
          tester.widget<TextField>(find.byType(TextField)).controller!.text,
          expectedText,
        );
      }
      // Refusal leaves autocomplete open. Dispose before draining its existing
      // 250ms search debounce; no send assertion depends on this cleanup pump.
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pump(const Duration(milliseconds: 250));
    });
  }

  testWidgets(
    'namesake picks show qualified chips over plain draft text and keep '
    'their own recipients (recommendation 4)',
    (tester) async {
      final hex64 = RegExp('[0-9a-f]{64}');
      List<String>? sent;
      String? sentText;
      await mountComposer(tester, members(), (
        text,
        keys, {
        mediaTags = const [],
      }) async {
        sentText = text;
        sent = keys;
      });
      await tester.enterText(find.byType(TextField), '@');
      await tester.pumpAndSettle();
      final labels = [
        for (final row in scoutRows().evaluate()) (row.widget as Text).data!,
      ];
      expect(labels, hasLength(2));
      expect(labels.toSet(), hasLength(2));
      expect(labels, everyElement(startsWith('Scout · ')));

      // One namesake picked: the chip carries the chooser's short label, the
      // draft text is the plain name, and the right person is notified. The
      // wire content stays bare: one key under the name needs no qualifier.
      await pick(tester, '@');
      final controller = tester
          .widget<TextField>(find.byType(TextField))
          .controller!;
      expect(controller.text, '@Scout ');
      expect(find.text('@${labels.first}'), findsOneWidget);
      expect(find.bySemanticsLabel('Mention: ${labels.first}'), findsOneWidget);
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sentText, '@Scout');
      expect(sent, [first]);

      // Both picked: no key ever reaches the composer text; each chip keeps
      // its label; the sent content qualifies both so bubbles tell them apart.
      await pick(tester, '@');
      await pick(tester, '@Scout @', last: true);
      expect(controller.text, '@Scout @Scout ');
      expect(controller.text, isNot(contains(hex64)));
      expect(find.text('@${labels.first}'), findsOneWidget);
      expect(find.text('@${labels.last}'), findsOneWidget);
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sentText, '@Scout ($first) @Scout ($second)');
      expect(sent, [first, second]);

      // Deleting the first of two identical chips removes its recipient, not
      // its namesake's.
      await pick(tester, '@');
      await pick(tester, '@Scout @', last: true);
      controller.selection = const TextSelection(
        baseOffset: 0,
        extentOffset: 7,
      );
      await tester.pump();
      tester.testTextInput.updateEditingValue(
        const TextEditingValue(
          text: '@Scout ',
          selection: TextSelection.collapsed(offset: 0),
        ),
      );
      await tester.pumpAndSettle();
      expect(controller.text, '@Scout ');
      expect(find.text('@${labels.last}'), findsOneWidget);
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sentText, '@Scout');
      expect(sent, [second]);

      // A chip inside code keeps its recipient but is sent as typed; only the
      // prose namesake is qualified.
      await pick(tester, '`ping @');
      await pick(tester, '`ping @Scout now` @', last: true);
      expect(controller.text, '`ping @Scout now` @Scout ');
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sentText, '`ping @Scout now` @Scout ($second)');
      expect(sent, [first, second]);
    },
  );

  testWidgets('unbound qualified text cannot notify a shorter member alias', (
    tester,
  ) async {
    List<String>? sent;
    await mountComposer(tester, [
      members().first,
    ], (_, keys, {mediaTags = const []}) async => sent = keys);
    await tester.enterText(find.byType(TextField), '@Scout ($second)');
    await tester.pumpAndSettle();
    await tester.tap(find.byIcon(BuzzIcons.arrowUp));
    await tester.pumpAndSettle();
    expect(sent, isEmpty);
  });

  testWidgets('ambiguous typed names fail visibly without clearing the draft', (
    tester,
  ) async {
    var sent = false;
    await mountComposer(tester, members(), (
      _,
      _, {
      mediaTags = const [],
    }) async {
      sent = true;
    });
    await tester.enterText(find.byType(TextField), '@Scout hello');
    await tester.pumpAndSettle();
    await tester.tap(find.byIcon(BuzzIcons.arrowUp));
    await tester.pumpAndSettle();
    expect(sent, isFalse);
    expect(
      tester.widget<TextField>(find.byType(TextField)).controller!.text,
      '@Scout hello',
    );
    expect(find.textContaining('is ambiguous'), findsOneWidget);
  });
}
