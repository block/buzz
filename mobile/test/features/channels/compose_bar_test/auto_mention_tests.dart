part of '../compose_bar_test.dart';

void autoMentionTests() {
  final agent = 'a' * 64;
  final otherAgent = 'b' * 64;
  final human = 'c' * 64;
  List<ChannelMember> roster() => [
    ChannelMember(
      pubkey: agent,
      displayName: 'Scout',
      role: 'bot',
      joinedAt: DateTime(2025),
    ),
    ChannelMember(
      pubkey: otherAgent,
      displayName: 'Scout',
      role: 'bot',
      joinedAt: DateTime(2025),
    ),
    ChannelMember(
      pubkey: human,
      displayName: 'Alice',
      role: 'member',
      joinedAt: DateTime(2025),
    ),
  ];
  Future<void> mount(
    WidgetTester tester,
    ComposeBarOnSend send, {
    String? thread = 'thread-1',
    List<String> initial = const [],
  }) async {
    await tester.pumpWidget(
      _buildComposeBar(
        uploadService: _testUploadService(nostr.Keys.generate().nsec),
        members: roster(),
        channels: [_makeCurrentChannel()],
        threadHeadId: thread,
        initialAgentMentionPubkeys: initial,
        onSend: send,
      ),
    );
    await tester.pumpAndSettle();
    if (find.byType(TextField).evaluate().isNotEmpty) return;
    if (find.text('Message\u2026').evaluate().isNotEmpty) {
      await _expandComposer(tester);
    } else {
      await tester.tap(find.textContaining('@Scout').first);
      await tester.pumpAndSettle();
    }
  }

  TextEditingController editor(WidgetTester tester) =>
      tester.widget<TextField>(find.byType(TextField)).controller!;
  Future<void> pick(
    WidgetTester tester,
    String query,
    String name, {
    bool last = false,
  }) async {
    await tester.enterText(find.byType(TextField), query);
    await tester.pumpAndSettle();
    await tester.tap(last ? find.text(name).last : find.text(name).first);
    await tester.pumpAndSettle();
  }

  testWidgets('auto-mention switch defaults off and persists from picker', (
    tester,
  ) async {
    await mount(tester, (_, _, {mediaTags = const []}) async {});
    await tester.enterText(find.byType(TextField), '@');
    await tester.pumpAndSettle();
    final toggle = find.byKey(const ValueKey('automatically-mention-agents'));
    expect(tester.widget<SwitchListTile>(toggle).value, isFalse);
    await tester.tap(toggle);
    await tester.pumpAndSettle();
    expect(tester.widget<SwitchListTile>(toggle).value, isTrue);
    expect(_testPrefs.getBool(automaticallyMentionAgentsKey), isTrue);
  });

  for (final thread in [null, 'thread-1']) {
    testWidgets('retains exact agent keys only in thread: $thread', (
      tester,
    ) async {
      await _testPrefs.setBool(automaticallyMentionAgentsKey, true);
      final sent = <(String, List<String>)>[];
      await mount(tester, (text, keys, {mediaTags = const []}) async {
        sent.add((text, keys));
      }, thread: thread);
      await pick(tester, '@', 'Scout');
      await pick(tester, '${editor(tester).text}@', 'Scout', last: true);
      final prefix = editor(tester).text;
      expect(prefix, contains(otherAgent));
      await pick(tester, '$prefix@Ali', 'Alice');
      await tester.enterText(
        find.byType(TextField),
        '${editor(tester).text}hello',
      );
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sent.single.$2, [agent, otherAgent, human]);
      expect(editor(tester).text, thread == null ? '' : prefix);
      if (thread != null) {
        await tester.enterText(
          find.byType(TextField),
          '${editor(tester).text}again',
        );
        await tester.pumpAndSettle();
        await tester.tap(find.byIcon(LucideIcons.arrowUp));
        await tester.pumpAndSettle();
        expect(sent.last.$2, [agent, otherAgent]);
      }
    });
  }

  testWidgets('deleted agent mention is not reinserted on next send', (
    tester,
  ) async {
    await _testPrefs.setBool(automaticallyMentionAgentsKey, true);
    await mount(tester, (_, _, {mediaTags = const []}) async {});
    await pick(tester, '@', 'Scout');
    await tester.enterText(find.byType(TextField), '@Scout hello');
    await tester.tap(find.byIcon(LucideIcons.arrowUp));
    await tester.pumpAndSettle();
    expect(editor(tester).text, '@Scout ');
    await tester.enterText(find.byType(TextField), 'no agent');
    await tester.pumpAndSettle();
    await tester.tap(find.byIcon(LucideIcons.arrowUp));
    await tester.pumpAndSettle();
    expect(editor(tester).text, isEmpty);
  });

  testWidgets('turning preference off removes only generated prefix', (
    tester,
  ) async {
    await _testPrefs.setBool(automaticallyMentionAgentsKey, true);
    await mount(
      tester,
      (_, _, {mediaTags = const []}) async {},
      initial: [agent],
    );
    await tester.enterText(find.byType(TextField), '@Scout authored body');
    await tester.pumpAndSettle();
    final container = ProviderScope.containerOf(
      tester.element(find.byType(ComposeBar)),
    );
    await container.read(autoMentionAgentsProvider.notifier).setEnabled(false);
    await tester.pumpAndSettle();
    expect(editor(tester).text, 'authored body');
  });

  testWidgets('thread scope switch does not leak retained recipients', (
    tester,
  ) async {
    await _testPrefs.setBool(automaticallyMentionAgentsKey, true);
    Future<void> send(
      String text,
      List<String> keys, {
      List<List<String>> mediaTags = const [],
    }) async {}
    await mount(tester, send, initial: [agent]);
    expect(editor(tester).text, '@Scout ');
    await mount(tester, send, thread: 'thread-2');
    expect(editor(tester).text, isEmpty);
    final container = ProviderScope.containerOf(
      tester.element(find.byType(ComposeBar)),
    );
    expect(
      container
          .read(composeDraftsProvider.notifier)
          .draftFor('channel-1:thread-1')
          ?.text,
      '@Scout ',
    );
  });

  for (final editFirst in [false, true]) {
    testWidgets('toggle off cleans surviving generated ranges: $editFirst', (
      tester,
    ) async {
      await _testPrefs.setBool(automaticallyMentionAgentsKey, true);
      await mount(
        tester,
        (_, _, {mediaTags = const []}) async {},
        initial: [agent, otherAgent],
      );
      final original = editor(tester).text;
      // Mutate only the first generated mention; the other must stay tracked.
      final first = editFirst ? '@ScouX ' : '';
      await tester.enterText(
        find.byType(TextField),
        '$first${original.substring('@Scout '.length)}',
      );
      await tester.pumpAndSettle();
      await tester.enterText(
        find.byType(TextField),
        '${editor(tester).text}authored body',
      );
      await tester.pumpAndSettle();
      final container = ProviderScope.containerOf(
        tester.element(find.byType(ComposeBar)),
      );
      await container
          .read(autoMentionAgentsProvider.notifier)
          .setEnabled(false);
      await tester.pumpAndSettle();
      expect(editor(tester).text, '${first}authored body');
      await tester.pump(const Duration(milliseconds: 300));
      await tester.pumpAndSettle();
    });
  }

  testWidgets('toggle off preserves manually retyped former auto mention', (
    tester,
  ) async {
    await _testPrefs.setBool(automaticallyMentionAgentsKey, true);
    await mount(
      tester,
      (_, _, {mediaTags = const []}) async {},
      initial: [agent],
    );
    await tester.enterText(find.byType(TextField), '');
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), '@Scout authored body');
    await tester.pumpAndSettle();
    final container = ProviderScope.containerOf(
      tester.element(find.byType(ComposeBar)),
    );
    await container.read(autoMentionAgentsProvider.notifier).setEnabled(false);
    await tester.pumpAndSettle();
    expect(editor(tester).text, '@Scout authored body');
  });

  testWidgets('failed auto-mention send restores full draft', (tester) async {
    await _testPrefs.setBool(automaticallyMentionAgentsKey, true);
    await mount(tester, (_, _, {mediaTags = const []}) async {
      throw Exception('offline');
    });
    await pick(tester, '@', 'Scout');
    await tester.enterText(find.byType(TextField), '@Scout hello');
    await tester.tap(find.byIcon(LucideIcons.arrowUp));
    await tester.pumpAndSettle();
    expect(editor(tester).text, '@Scout hello');
    expect(find.text('offline'), findsOneWidget);
    final container = ProviderScope.containerOf(
      tester.element(find.byType(ComposeBar)),
    );
    await container.read(autoMentionAgentsProvider.notifier).setEnabled(false);
    await tester.pumpAndSettle();
    expect(editor(tester).text, '@Scout hello');
  });

  testWidgets(
    'thread seed uses signed keys and excludes humans; deletion sticks',
    (tester) async {
      await _testPrefs.setBool(automaticallyMentionAgentsKey, true);
      final sent = <List<String>>[];
      await mount(tester, (_, keys, {mediaTags = const []}) async {
        sent.add(keys);
      }, initial: [agent, human]);
      expect(editor(tester).text, '@Scout ');
      await tester.enterText(find.byType(TextField), 'no mention');
      await tester.pumpAndSettle();
      await tester.tap(find.byIcon(LucideIcons.arrowUp));
      await tester.pumpAndSettle();
      expect(sent.single, isEmpty);
      expect(editor(tester).text, isEmpty);
    },
  );
}
