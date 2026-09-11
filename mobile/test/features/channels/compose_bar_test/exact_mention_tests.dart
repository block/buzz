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

  Future<void> pick(
    WidgetTester tester,
    String text, {
    bool last = false,
  }) async {
    await tester.enterText(find.byType(TextField), text);
    await tester.pumpAndSettle();
    await tester.tap(last ? find.text('Scout').last : find.text('Scout').first);
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
    'malformed',
    'tainted',
    'non-member human',
    'non-member denied agent',
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
      expect(saved.single['mention_keys'], {'Scout': first});
      // Dispose the actual provider state, not just the text controller.
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
      if (scenario.startsWith('map ')) saved.single['mention_keys'] = null;
      if (scenario == 'legacy') saved.single.remove('mention_keys');
      if (scenario == 'malformed') {
        saved.single['mention_keys'] = {'Scout': 'not-a-key'};
      }
      if (scenario == 'tainted') {
        saved.single['mention_keys'] = {
          'Scout': {'pubkey': second, 'is_agent': true},
        };
      }
      await _testPrefs.setString(prefsKey, jsonEncode(saved));
      await mount([
        if (![
          'removed',
          'non-member human',
          'non-member denied agent',
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
                ownerPubkey: scenario == 'non-member denied agent'
                    ? second
                    : null,
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
        expectedText = '@Scout hello';
        await tester.enterText(find.byType(TextField), expectedText);
        await tester.pumpAndSettle();
        expect(
          jsonDecode(_testPrefs.getString(prefsKey)!).single['mention_keys'],
          {'': ''},
        );
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
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();
      if (scenario == 'non-member human') {
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
        'non-member human',
        'map reset',
        'map plain',
        'map reselected',
      ].contains(scenario)) {
        final event = messages.single;
        expect(event['content'], expectedText.trim());
        expect((event['tags'] as List).where((t) => t[0] == 'p').toList(), [
          if (scenario != 'non-member human' && scenario != 'map plain')
            [
              'p',
              ['legacy', 'map reselected'].contains(scenario) ? second : first,
            ],
        ]);
        if (scenario == 'non-member human') {
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
    'same-name picker selections retain exact recipients through removal',
    (tester) async {
      List<String>? sent;
      await mountComposer(
        tester,
        members(),
        (_, keys, {mediaTags = const []}) async => sent = keys,
      );
      await pick(tester, '@');
      final controller = tester
          .widget<TextField>(find.byType(TextField))
          .controller!;
      expect(controller.text, '@Scout ');
      await pick(tester, '@Scout @', last: true);
      expect(controller.text, '@Scout @Scout ($second) ');
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sent, [first, second]);
      await pick(tester, '@');
      await pick(tester, '@Scout @', last: true);
      await tester.enterText(find.byType(TextField), '@Scout ($second) ');
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sent, [second]);
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
    await tester.tap(find.byIcon(LucideIcons.arrowUp));
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
