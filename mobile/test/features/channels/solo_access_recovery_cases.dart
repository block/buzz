part of 'channel_access_test.dart';

class _SharingRelay extends _Relay {
  String destinationRole = 'owner';
  String creator = _keys.public;
  bool archived = false;
  bool loseAccessOnce = false;
  bool loseLinkOnce = false;

  @override
  NostrEvent event(int kind) {
    final original = super.event(kind);
    if (kind != 39000) return original;
    return NostrEvent.fromJson(
      signNostrEvent(
        secretKey: _relayKeys.secret,
        createdAt: revision,
        kind: kind,
        content: '',
        tags: [
          for (final tag in original.tags)
            if (tag.first != 'P') tag,
          ['P', creator],
          if (archived) ['archived', 'true'],
        ],
      ).toMap(),
    );
  }

  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final result = await super.publish(event, timeout: timeout);
    if (event.kind == 9002 && loseAccessOnce) {
      loseAccessOnce = false;
      throw TimeoutException('Access response lost');
    }
    if (event.kind == 9 && loseLinkOnce) {
      loseLinkOnce = false;
      throw TimeoutException('Link response lost');
    }
    return result;
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filter.tags['#d']?.contains('destination') == true) {
      final kind = filter.kinds.single;
      return [
        NostrEvent.fromJson(
          signNostrEvent(
            secretKey: _relayKeys.secret,
            kind: kind,
            content: '',
            tags: [
              ['d', 'destination'],
              if (kind == 39000) ...[
                ['name', 'team'],
                ['t', 'stream'],
                ['public'],
                ['P', _keys.public],
              ],
              if (kind == 39002) ...[
                if (destinationRole.isNotEmpty)
                  ['p', _keys.public, '', destinationRole],
                // Sharing a large audience never enrolls anyone or reads profiles.
                for (var i = 0; i < 102; i++)
                  ['p', i.toRadixString(16).padLeft(64, '0'), '', 'guest'],
              ],
            ],
          ).toMap(),
        ),
      ];
    }
    return super.fetchHistory(filter, timeout: timeout);
  }
}

Channel _soloChannel() => Channel(
  id: _id,
  name: 'Research chat',
  channelType: 'stream',
  labels: const ['buzz.solo', 'project.work'],
  visibility: 'private',
  description: '',
  createdBy: _keys.public,
  createdAt: DateTime(2026),
  memberCount: 2,
  isMember: true,
);

Future<MeSync> _recoverySync(_Relay relay) async {
  SharedPreferences.setMockInitialValues({});
  return MeSync(
    relay,
    RelayConfig(baseUrl: 'https://access.example', nsec: _keys.nsec),
    prefs: await SharedPreferences.getInstance(),
  );
}

Future<void> _showRecoveryWidget(
  WidgetTester tester,
  ProviderContainer container,
  Widget sheet,
) async {
  await tester.pumpWidget(
    UncontrolledProviderScope(
      container: container,
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(body: sheet),
      ),
    ),
  );
  await _settleRecovery(tester);
}

void soloAccessRecoveryCases() {
  test(
    'offline before any roster keeps the cached-list fallback available',
    () async {
      final relay = _Relay();
      final container = _container(relay);
      addTearDown(container.dispose);
      container.read(relaySessionProvider);
      relay.disconnect();
      final subscription = container.listen(
        channelContributionProvider(_id),
        (_, _) {},
      );
      addTearDown(subscription.close);
      await container.read(channelMembersProvider(_id).future);
      expect(container.read(channelContributionProvider(_id)), isNull);
    },
  );

  testWidgets(
    'sharing confirms community reading, recovers access and links without enrolling anyone',
    (tester) async {
      final relay = _SharingRelay()
        ..loseAccessOnce = true
        ..loseLinkOnce = true;
      final sync = await _recoverySync(relay);
      final container = _container(relay, sync: sync, shareChoices: true);
      addTearDown(container.dispose);
      await _showRecoveryWidget(
        tester,
        container,
        SoloShareSheet(channel: _soloChannel()),
      );
      await tester.enterText(find.byType(TextField), 'team');
      await _settleRecovery(tester);
      await tester.tap(find.text('#team'));
      await _settleRecovery(tester);
      expect(
        find.textContaining('This chat will change to View only.'),
        findsOneWidget,
      );
      expect(
        find.textContaining(
          'Everyone in this community can read its full history, Canvas, workspace folders, and agent instructions.',
        ),
        findsOneWidget,
      );
      expect(
        find.textContaining('No one is added to the chat'),
        findsOneWidget,
      );
      await tester.tap(find.text('Cancel'));
      await _settleRecovery(tester);
      expect(MeChatShare.pending(sync, _id), isNull);
      expect(relay.published, isEmpty);

      await tester.tap(find.text('#team'));
      await _settleRecovery(tester);
      await tester.tap(find.widgetWithText(FilledButton, 'Share'));
      await _settleRecovery(tester);
      final messageId = MeChatShare.pending(sync, _id)!.message.id;
      expect(relay.visibility, 'open');
      expect(relay.joinRole, 'guest');
      expect(relay.published.map((e) => e.kind), [9002]);
      // The lost access response is checked on retry, never republished.
      await tester.tap(find.text('Retry share'));
      await _settleRecovery(tester);
      expect(find.textContaining('This chat will change'), findsNothing);
      await tester.tap(find.widgetWithText(FilledButton, 'Share'));
      await _settleRecovery(tester);
      expect(relay.published.map((e) => e.kind), [9002, 9]);
      expect(MeChatShare.pending(sync, _id)!.message.id, messageId);

      // A chat made Private while View-only consent is visible cannot be reopened.
      await tester.tap(find.text('Retry share'));
      await _settleRecovery(tester);
      relay.visibility = 'private';
      relay.revision++;
      await tester.tap(find.widgetWithText(FilledButton, 'Share'));
      await _settleRecovery(tester);
      expect(
        find.text('Chat access changed. Review sharing again.'),
        findsOneWidget,
      );
      expect(relay.visibility, 'private');
      expect(relay.published.map((e) => e.kind), [9002, 9]);
      // Fresh Private consent allows opening again; uncertain link ID is retained.
      await tester.tap(find.text('Retry share'));
      await _settleRecovery(tester);
      expect(
        find.textContaining('This chat will change to View only.'),
        findsOneWidget,
      );
      await tester.tap(find.widgetWithText(FilledButton, 'Share'));
      await _settleRecovery(tester);
      expect(MeChatShare.pending(sync, _id), isNull);
      expect(relay.published.map((e) => e.kind), [9002, 9, 9002, 9]);
      expect(relay.published.where((e) => e.kind == 9).map((e) => e.id), [
        messageId,
        messageId,
      ]);
      expect(relay.published.last.content, contains('buzz://channel/$_id'));
      expect(relay.published.last.channelId, 'destination');
      expect(relay.labels, ['buzz.solo', 'project.work']);
      expect(relay.actorRole, 'owner');
      expect(relay.targetRole, 'bot');
      expect(relay.destinationRole, 'owner');
      expect(relay.profileReads, 0);
      final edits = relay.published.where((e) => e.kind == 9002).toList();
      expect(edits.first.tags, [
        ['h', _id],
        ['visibility', 'open'],
        ['join_role', 'guest'],
      ]);
      expect(edits.last.tags, [
        ['h', _id],
        ['visibility', 'open'],
      ]);
      await tester.pumpWidget(const SizedBox());
    },
  );
  testWidgets(
    'sharing uses current source and destination permissions and preserves open policy',
    (tester) async {
      for (final row in <(String, String, bool, bool, String, bool)>[
        // visibility, source role, creator, archived, destination role, allowed
        ('private', 'owner', true, false, 'member', true),
        ('private', 'admin', true, false, 'member', true),
        ('private', 'admin', false, false, 'member', false),
        ('private', 'member', true, false, 'member', false),
        ('open', 'member', false, false, 'member', true),
        ('open', 'bot', false, false, 'member', true),
        ('open', 'guest', true, false, 'member', false),
        ('open', '', true, false, 'member', false),
        ('open', 'owner', true, true, 'member', false),
        ('private', 'owner', true, false, 'guest', false),
        ('private', 'owner', true, false, '', false),
      ]) {
        for (final policy in ['member', 'guest']) {
          final relay = _SharingRelay()
            ..visibility = row.$1
            ..joinRole = policy
            ..actorRole = row.$2
            ..creator = row.$3 ? _keys.public : _target
            ..archived = row.$4
            ..destinationRole = row.$5;
          final sync = await _recoverySync(relay);
          final container = _container(relay, sync: sync, shareChoices: true);
          await _showRecoveryWidget(
            tester,
            container,
            SoloShareSheet(channel: _soloChannel()),
          );
          await tester.enterText(find.byType(TextField), 'team');
          await _settleRecovery(tester);
          await tester.tap(find.text('#team'));
          await _settleRecovery(tester);
          if (row.$6 && (row.$1 == 'private' || policy == 'guest')) {
            await tester.tap(find.widgetWithText(FilledButton, 'Share'));
            await _settleRecovery(tester);
          }
          expect(
            relay.published.map((e) => e.kind),
            row.$6 ? (row.$1 == 'private' ? [9002, 9] : [9]) : [],
            reason: '$row / $policy',
          );
          expect(
            relay.joinRole,
            row.$6 && row.$1 == 'private' ? 'guest' : policy,
          );
          expect(relay.actorRole, row.$2);
          expect(relay.targetRole, 'bot');
          await tester.pumpWidget(const SizedBox());
          container.dispose();
        }
      }
    },
  );
  testWidgets('sharing rechecks demotions and metadata after confirmation', (
    tester,
  ) async {
    for (final change in ['source', 'destination', 'metadata']) {
      final relay = _SharingRelay();
      final sync = await _recoverySync(relay);
      final container = _container(relay, sync: sync, shareChoices: true);
      await _showRecoveryWidget(
        tester,
        container,
        SoloShareSheet(channel: _soloChannel()),
      );
      await tester.enterText(find.byType(TextField), 'team');
      await _settleRecovery(tester);
      await tester.tap(find.text('#team'));
      await _settleRecovery(tester);
      if (change == 'source') relay.actorRole = 'guest';
      if (change == 'destination') relay.destinationRole = 'guest';
      relay.revision++;
      await tester.tap(find.widgetWithText(FilledButton, 'Share'));
      await _settleRecovery(tester);
      expect(relay.visibility, 'private');
      expect(relay.published, isEmpty, reason: change);
      await tester.pumpWidget(const SizedBox());
      container.dispose();
    }
  });
  testWidgets(
    'an interrupted share can be kept or cancelled without another grant or post',
    (tester) async {
      final relay = _Relay();
      final sync = await _recoverySync(relay);
      final intent = MeChatShare.create(
        sync,
        source: _id,
        name: 'Research',
        destination: 'gone',
        destinationName: 'deleted channel',
      );
      await expectLater(
        intent.complete(
          sync,
          prepare: (_) async => throw StateError('Destination deleted'),
          send: (_) async => fail('No link may be sent'),
        ),
        throwsStateError,
      );
      final container = _container(relay, sync: sync);
      addTearDown(container.dispose);
      await _showRecoveryWidget(
        tester,
        container,
        SoloShareSheet(channel: _soloChannel()),
      );
      await tester.tap(find.text('Cancel share'));
      await _settleRecovery(tester);
      expect(
        find.textContaining('possibly-posted link may remain'),
        findsOneWidget,
      );
      await tester.tap(find.text('Keep share'));
      await _settleRecovery(tester);
      expect(MeChatShare.pending(sync, _id), isNotNull);
      await tester.tap(find.text('Cancel share'));
      await _settleRecovery(tester);
      await tester.tap(find.widgetWithText(FilledButton, 'Cancel share'));
      await _settleRecovery(tester);
      expect(MeChatShare.pending(sync, _id), isNull);
      expect(find.widgetWithText(TextField, 'Search channels'), findsOneWidget);
      expect(relay.published, isEmpty);
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'conflicting conversion can be discarded only after confirmation',
    (tester) async {
      final relay = _Relay();
      final sync = await _recoverySync(relay);
      final intent = MeChatConversion.create(
        sync,
        channel: _id,
        visibility: 'open',
        joinRole: 'guest',
        labels: [],
      );
      await expectLater(
        intent.complete(
          sync,
          ensureConverted: () async => throw StateError('Conflict'),
        ),
        throwsStateError,
      );
      final container = _container(relay, sync: sync);
      addTearDown(container.dispose);
      await _showRecoveryWidget(
        tester,
        container,
        SoloConversionSheet(channel: _soloChannel()),
      );
      await tester.tap(find.text('Finish conversion'));
      await _settleRecovery(tester);
      expect(find.textContaining('Chat access changed.'), findsOneWidget);
      await tester.tap(find.text('Discard conversion'));
      await _settleRecovery(tester);
      await tester.tap(find.text('Keep conversion'));
      await _settleRecovery(tester);
      expect(MeChatConversion.pending(sync, _id), isNotNull);
      await tester.tap(find.text('Discard conversion'));
      await _settleRecovery(tester);
      await tester.tap(find.widgetWithText(FilledButton, 'Discard conversion'));
      await _settleRecovery(tester);
      expect(MeChatConversion.pending(sync, _id), isNull);
      expect(relay.published, isEmpty);
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets(
    'Public conversion discloses posting to agents and can be cancelled',
    (tester) async {
      final relay = _Relay();
      final sync = await _recoverySync(relay);
      final container = _container(relay, sync: sync);
      addTearDown(container.dispose);
      await _showRecoveryWidget(
        tester,
        container,
        SoloConversionSheet(channel: _soloChannel()),
      );
      await tester.tap(find.byType(DropdownButtonFormField<String>));
      await _settleRecovery(tester);
      await tester.tap(find.text('Public').last);
      await _settleRecovery(tester);
      await tester.tap(find.text('Convert to channel'));
      await _settleRecovery(tester);
      expect(
        find.textContaining(
          'Everyone in the community can join, read and post, including to your agents.',
        ),
        findsOneWidget,
      );
      await tester.tap(find.text('Cancel'));
      await _settleRecovery(tester);
      expect(MeChatConversion.pending(sync, _id), isNull);
      expect(relay.published, isEmpty);
      await tester.pumpWidget(const SizedBox());
    },
  );

  testWidgets('a disabled Convert button explains the required permission', (
    tester,
  ) async {
    final relay = _Relay()..actorRole = 'member';
    final sync = await _recoverySync(relay);
    final container = _container(relay, sync: sync);
    addTearDown(container.dispose);
    await _showRecoveryWidget(
      tester,
      container,
      SoloConversionSheet(channel: _soloChannel()),
    );
    expect(
      find.text(
        'Only the chat creator with owner or admin access can convert this chat.',
      ),
      findsOneWidget,
    );
    expect(
      tester
          .widget<FilledButton>(
            find.widgetWithText(FilledButton, 'Convert to channel'),
          )
          .onPressed,
      isNull,
    );
    await tester.pumpWidget(const SizedBox());
  });

  test(
    'the freshest roster survives disconnect after demotion and promotion',
    () async {
      final relay = _Relay()..actorRole = 'guest';
      final container = _container(relay);
      addTearDown(container.dispose);
      final subscription = container.listen(
        channelContributionProvider(_id),
        (_, _) {},
      );
      addTearDown(subscription.close);
      await container.read(channelMembersProvider(_id).future);
      expect(container.read(channelContributionProvider(_id)), isFalse);
      relay.disconnect();
      expect(container.read(channelContributionProvider(_id)), isFalse);
      relay.actorRole = 'member';
      relay.connect();
      relay.revision++;
      container.invalidate(channelMembersProvider(_id));
      await container.read(channelMembersProvider(_id).future);
      expect(container.read(channelContributionProvider(_id)), isTrue);
      relay.disconnect();
      expect(container.read(channelContributionProvider(_id)), isTrue);
    },
  );

  testWidgets('an agent Guest returns to Bot and has no Admin option', (
    tester,
  ) async {
    final relay = _Relay()
      ..labels = []
      ..targetRole = 'guest';
    final container = _container(relay, owners: {_target: _keys.public});
    addTearDown(container.dispose);
    await _showRecoveryWidget(
      tester,
      container,
      const MemberAccessControls(
        channelId: _id,
        target: _target,
        label: 'Agent',
      ),
    );
    expect(find.text('Admin'), findsNothing);
    await tester.tap(find.text('Participating'));
    await _settleRecovery(tester);
    await tester.tap(find.widgetWithText(FilledButton, 'Change access'));
    await _settleRecovery(tester);
    expect(relay.published.single.getTagValue('role'), 'bot');
    await tester.pumpWidget(const SizedBox());
  });

  testWidgets('member removal uses the native iOS destructive confirmation', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    const bridge = MethodChannel('buzz/confirmation_dialog');
    final calls = <MethodCall>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(bridge, (call) async {
          calls.add(call);
          return false;
        });
    addTearDown(() {
      debugDefaultTargetPlatformOverride = null;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(bridge, null);
    });
    final relay = _Relay()..labels = [];
    final container = _container(relay);
    addTearDown(container.dispose);
    await _showRecoveryWidget(
      tester,
      container,
      const MemberAccessControls(
        channelId: _id,
        target: _target,
        label: 'Agent',
      ),
    );
    await tester.tap(find.text('Remove from channel'));
    await _settleRecovery(tester);
    expect(calls, hasLength(1));
    expect(calls.single.arguments, containsPair('confirmLabel', 'Remove'));
    expect(relay.published, isEmpty);
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });

  test('readable access failures remove runtime prefixes', () {
    expect(
      accessErrorMessage(StateError('Access changed.')),
      'Access changed.',
    );
    expect(accessErrorMessage(Exception('Not allowed.')), 'Not allowed.');
    expect(
      accessErrorMessage(TimeoutException('network')),
      'The request timed out. Check your connection and try again.',
    );
  });
}

class _ShareChannels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [
    Channel(
      id: 'destination',
      name: 'team',
      channelType: 'stream',
      visibility: 'open',
      description: '',
      createdBy: _keys.public,
      createdAt: DateTime(2026),
      memberCount: 2,
      isMember: true,
      viewerRole: 'member',
    ),
  ];
}

// Busy indicators continue animating behind confirmation dialogs. Pump bounded
// frames, then assert actual dialog/journal state instead of waiting for idle.
Future<void> _settleRecovery(WidgetTester tester) async {
  await tester.pump();
  for (var i = 0; i < 5; i++) {
    await tester.pump(const Duration(milliseconds: 100));
  }
}
