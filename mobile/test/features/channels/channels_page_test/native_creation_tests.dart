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
      final users = [
        for (var i = 0; i < 10; i++)
          DirectoryUser(pubkey: 'person$i', displayName: 'Person $i'),
      ];
      final actions = _NativeDmActions();
      await tester.pumpWidget(
        buildPage([
          channelsProvider.overrideWith(() => _FakeNotifier([])),
          relayDirectoryUsersProvider.overrideWith((ref) async => users),
          relayDirectorySearchProvider.overrideWith(
            (ref, query) async => users
                .where((u) => u.label.toLowerCase().contains(query))
                .toList(),
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
      expect(states.last['title'], 'New message');
      expect(states.last['confirmWithCheckmark'], true);
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
      expect((states.last['users'] as List).single['pubkey'], 'person2');
      await event('toggle', {'pubkey': 'person2'});
      await event('query', {'value': ''});
      for (final user in users) {
        await event('toggle', {'pubkey': user.pubkey});
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
