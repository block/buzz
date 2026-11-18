part of '../channels_page_test.dart';

void registerNativeCreationTests(Widget Function(List<Override>) buildPage) {
  testWidgets(
    'native DM picker searches, limits selection and preserves it on failure',
    (tester) async {
      const bridge = MethodChannel('buzz/add_members_sheet');
      final states = <Map<Object?, Object?>>[];
      final messenger = tester.binding.defaultBinaryMessenger;
      messenger.setMockMethodCallHandler(bridge, (call) async {
        if (call.method != 'dismiss') {
          states.add(Map<Object?, Object?>.from(call.arguments as Map));
        }
        return null;
      });
      addTearDown(() {
        messenger.setMockMethodCallHandler(bridge, null);
        debugDefaultTargetPlatformOverride = null;
      });
      final people = {for (var i = 0; i < 10; i++) _dmKey(i): 'Person $i'};
      final actions = _NativeDmActions();
      await tester.pumpWidget(
        buildPage([
          channelsProvider.overrideWith(() => _FakeNotifier([])),
          relaySessionProvider.overrideWith(
            () => RecipientDirectorySession(people),
          ),
          channelActionsProvider.overrideWithValue(actions),
        ]),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byTooltip('Create'));
      await tester.pumpAndSettle();
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      await tester.tap(find.text('New DM'));
      await tester.pumpAndSettle();
      expect(find.byType(BottomSheet), findsNothing);
      expect(states.last['title'], 'New DM');
      expect(states.last['confirmWithCheckmark'], true);
      expect(states.last['clearQueryOnPick'], true);
      final session = states.last['session'];
      Future<void> event(
        String method, [
        Map<String, Object?> data = const {},
      ]) async {
        await messenger.handlePlatformMessage(
          bridge.name,
          const StandardMethodCodec().encodeMethodCall(
            MethodCall(method, {'session': session, ...data}),
          ),
          (_) {},
        );
        await tester.pump();
        if (method == 'query') {
          await tester.pump(const Duration(milliseconds: 250));
        }
        await tester.pumpAndSettle();
      }

      await event('query', {'value': 'person 2'});
      final found = (states.last['users'] as List).single as Map;
      expect(found['pubkey'], _dmKey(2));
      // People carry no key or handle line, as in the mention chooser.
      expect(found['detail'], isNull);
      await event('toggle', {'pubkey': _dmKey(2)});
      await event('query', {'value': 'Person'});
      // Two taps before Flutter rebuilds still never list a person twice.
      for (var tap = 0; tap < 2; tap++) {
        await messenger.handlePlatformMessage(
          bridge.name,
          const StandardMethodCodec().encodeMethodCall(
            MethodCall('toggle', {'session': session, 'pubkey': _dmKey(3)}),
          ),
          (_) {},
        );
      }
      await tester.pumpAndSettle();
      final picked = [
        for (final user in states.last['selected'] as List)
          (user as Map)['pubkey'],
      ];
      expect(picked.toSet(), hasLength(picked.length));
      for (final key in people.keys) {
        await event('toggle', {'pubkey': key});
      }
      expect(states.last['selected'] as List, hasLength(8));
      await event('submit');
      expect(states.last['error'], contains('Try again'));
      expect(states.last['selected'] as List, hasLength(8));
      expect(actions.calls.single, hasLength(8));
      await event('closed');
      expect(
        Navigator.of(tester.element(find.byType(ChannelsPage))).canPop(),
        isFalse,
      );
      await tester.pumpWidget(const SizedBox.shrink());
      debugDefaultTargetPlatformOverride = null;
    },
  );
}

class _NativeDmActions implements ChannelActions {
  final calls = <List<String>>[];
  @override
  Future<Channel> openDm({
    required List<String> pubkeys,
    bool waitForChannelList = true,
  }) async {
    calls.add(pubkeys);
    throw StateError('Try again');
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
