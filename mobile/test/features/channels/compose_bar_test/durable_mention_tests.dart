part of '../compose_bar_test.dart';

void durableMentionTests() {
  testWidgets(
    'exact draft recipients survive restart and failed-send recovery',
    (tester) async {
      final keys = nostr.Keys.generate();
      final first = 'a' * 64;
      final second = 'b' * 64;
      var renamed = false;
      var fail = true;
      List<String>? sent;
      Widget build({String? thread}) => _buildComposeBar(
        threadHeadId: thread,
        uploadService: _testUploadService(keys.nsec),
        relayConfig: () => _SwitchableRelayConfigNotifier(
          RelayConfig(baseUrl: 'http://localhost:3000', nsec: keys.nsec),
        ),
        channels: [_makeCurrentChannel()],
        members: [
          for (final key in [first, second])
            ChannelMember(
              pubkey: key,
              displayName: renamed ? 'Renamed' : 'Scout',
              role: 'member',
              joinedAt: DateTime(2025),
            ),
        ],
        onSend: (_, mentions, {mediaTags = const []}) async {
          sent = mentions;
          if (fail) throw Exception('relay rejected');
        },
      );
      await tester.pumpWidget(build());
      await _expandComposer(tester);
      await tester.enterText(find.byType(TextField), '@');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Scout').first);
      await tester.pumpAndSettle();
      await tester.enterText(find.byType(TextField), '@Scout @');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Scout').last);
      await tester.pumpAndSettle();
      final draft = tester
          .widget<TextField>(find.byType(TextField))
          .controller!
          .text;
      expect(draft, '@Scout @Scout ($second) ');

      // Same mounted composer, different thread and back: old listeners may
      // not erase the original persisted bindings while restoring another key.
      await tester.pumpWidget(build(thread: 'other'));
      await tester.pumpAndSettle();
      await tester.pumpWidget(build());
      await tester.pumpAndSettle();
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        draft,
      );
      await tester.pumpWidget(const SizedBox.shrink());
      renamed = true;
      await tester.pumpWidget(build());
      await tester.pumpAndSettle();
      await tester.tap(find.text(draft.trim()));
      await tester.pumpAndSettle();
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        draft,
      );
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sent, [first, second]);
      expect(
        tester.widget<TextField>(find.byType(TextField)).controller!.text,
        draft,
      );

      // Recovery must persist the bindings before notifying text listeners.
      await tester.pumpWidget(const SizedBox.shrink());
      fail = false;
      sent = null;
      await tester.pumpWidget(build());
      await tester.pumpAndSettle();
      await tester.tap(find.text(draft.trim()));
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sent, [first, second]);
    },
  );

  for (final fallback in ['unavailable', 'human']) {
    testWidgets('saved agent loses provenance across restart: $fallback', (
      tester,
    ) async {
      final signer = nostr.Keys.generate();
      final agent = 'a' * 64;
      final events = <Map<String, dynamic>>[];
      var eligible = true;
      Future<void> mount() async {
        await tester.pumpWidget(
          _buildComposeBar(
            uploadService: _testUploadService(signer.nsec),
            currentPubkey: signer.public,
            relayConfig: () => _SwitchableRelayConfigNotifier(
              RelayConfig(baseUrl: 'https://relay.example', nsec: signer.nsec),
            ),
            channels: [_makeCurrentChannel(), _makeSharedMemberChannel()],
            relayAgents: [
              if (eligible)
                AgentDirectoryEntry(
                  pubkey: agent,
                  displayName: 'Helper Bot',
                  respondTo: 'anyone',
                  channelIds: const ['shared-channel'],
                ),
            ],
            onSend: (text, keys, {mediaTags = const []}) async {
              final container = ProviderScope.containerOf(
                tester.element(find.byType(ComposeBar)),
              );
              final session = container.read(relaySessionProvider.notifier);
              await SendMessage(
                signedEventRelay: SignedEventRelay(
                  session: session,
                  nsec: signer.nsec,
                ),
                fetchMembers: (_) async => const [],
                readUserCache: () => const {},
                addLocalMessage: (_, _) {},
                completeLocalMessage: (_, _) {},
                removeLocalMessage: (_, _) {},
              )(
                channelId: 'channel-1',
                content: text,
                mentionPubkeys: keys,
                mediaTags: mediaTags,
              );
            },
          ),
        );
        await tester.pumpAndSettle();
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
        if (!eligible && fallback == 'human') {
          container
              .read(userCacheProvider.notifier)
              .put(UserProfile(pubkey: agent, displayName: 'Former Helper'));
          await tester.pumpAndSettle();
        }
      }

      await mount();
      await _expandComposer(tester);
      await tester.enterText(find.byType(TextField), '@');
      await tester.pumpAndSettle();
      await tester.tap(find.text('Helper Bot'));
      await tester.pumpAndSettle();
      final prefsKey =
          'compose_drafts_v1:https://relay.example:${signer.public}';
      final saved = jsonDecode(_testPrefs.getString(prefsKey)!) as List;
      expect(saved.single['mention_keys'], {'Helper Bot': agent});
      expect(saved.single['text'], '@Helper Bot ');
      // The parent deliberately persists no agent bit to trust after restart.
      expect(saved.single.containsKey('is_agent'), isFalse);
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pumpAndSettle();
      eligible = false;
      await mount();
      await tester.tap(find.text('@Helper Bot'));
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(events.where((e) => e['kind'] == 9000), isEmpty);
      if (fallback == 'human') {
        expect(
          find.textContaining('Former Helper is not in this channel'),
          findsOneWidget,
        );
        await tester.tap(find.text('Do nothing'));
        await tester.pumpAndSettle();
        final message = events.singleWhere(
          (e) => e['kind'] == EventKind.streamMessage,
        );
        expect((message['tags'] as List).where((t) => t[0] == 'p'), isEmpty);
        expect(
          (message['tags'] as List).where((t) => t[0] == 'mention').toList(),
          [
            ['mention', agent],
          ],
        );
        expect(message['sig'], matches(RegExp(r'^[0-9a-f]{128}$')));
      } else {
        expect(
          events.where((e) => e['kind'] == EventKind.streamMessage),
          isEmpty,
        );
        expect(
          find.textContaining('Saved mention is no longer available'),
          findsOneWidget,
        );
        expect(
          tester.widget<TextField>(find.byType(TextField)).controller!.text,
          '@Helper Bot ',
        );
      }
      expect(events.where((e) => e['kind'] == 9000), isEmpty);
      await tester.pumpWidget(const SizedBox.shrink());
      await tester.pump(const Duration(milliseconds: 250));
    });
  }
}
