import 'dart:async';
import 'dart:ui' as ui;

import 'package:buzz/features/channels/add_members_sheet.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/recipient_directory_session.dart';

final _alice = 'a' * 64;
final _bob = 'b' * 64;
final _existing = 'e' * 64;
final _avatar =
    'data:image/svg+xml,${Uri.encodeComponent('<svg xmlns="http://www.w3.org/2000/svg" width="72" height="72"><rect width="72" height="72" fill="#0000ff"/></svg>')}';
const _bridge = MethodChannel('buzz/add_members_sheet');

class _Cache extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => const {};
  @override
  Future<bool> preload(List<String> pubkeys) async => true;
}

class _Lifecycle extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

class _Actions extends Fake implements ChannelActions {
  final calls = <List<String>>[];
  bool failBob = false;
  @override
  Future<void> addMembers({
    required String channelId,
    required List<String> pubkeys,
    String role = 'member',
  }) async {
    calls.add(pubkeys);
    if (failBob) throw AddMembersException({_bob: 'Try again'});
  }
}

Future<void> _pump(
  WidgetTester tester,
  _Actions actions, {
  Map<String, String>? people,
  Set<String>? agents,
}) async {
  await tester.pumpWidget(
    ProviderScope(
      overrides: [
        userCacheProvider.overrideWith(_Cache.new),
        channelActionsProvider.overrideWithValue(actions),
        channelMembersProvider('channel').overrideWith((ref) async => []),
        appLifecycleProvider.overrideWith(_Lifecycle.new),
        relaySessionProvider.overrideWith(
          () => RecipientDirectorySession(
            people ?? {_alice: 'Alice', _bob: 'Bob', _existing: 'Existing'},
            agents: agents ?? {_bob},
            pictures: {_bob: _avatar},
          ),
        ),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Scaffold(
          body: Builder(
            builder: (context) => TextButton(
              onPressed: () => showAddChannelMembersSheet(
                context: context,
                channelId: 'channel',
                existingPubkeys: {_existing},
              ),
              child: const Text('Open picker'),
            ),
          ),
        ),
      ),
    ),
  );
  await tester.tap(find.text('Open picker'));
  await tester.pumpAndSettle();
}

/// Lets a debounced picker search finish.
Future<void> _settleSearch(WidgetTester tester) async {
  await tester.pump();
  await tester.pump(const Duration(milliseconds: 300));
  await tester.pumpAndSettle();
}

Future<void> _event(
  WidgetTester tester,
  String session,
  String method, [
  Map<String, Object?> values = const {},
]) async {
  final response = Completer<void>();
  await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
    _bridge.name,
    const StandardMethodCodec().encodeMethodCall(
      MethodCall(method, {'session': session, ...values}),
    ),
    (_) => response.complete(),
  );
  await response.future;
  await tester.pumpAndSettle();
}

void main() {
  for (final platform in [TargetPlatform.iOS, TargetPlatform.android]) {
    testWidgets(
      'same-name agents remain distinct in ${platform.name} member choices',
      (tester) async {
        debugDefaultTargetPlatformOverride = platform;
        final states = <Map<Object?, Object?>>[];
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(_bridge, (call) async {
              if (call.method != 'dismiss') {
                states.add(Map<Object?, Object?>.from(call.arguments as Map));
              }
              return null;
            });
        try {
          final actions = _Actions();
          await _pump(
            tester,
            actions,
            people: {_alice: 'Scout', _bob: 'Scout'},
            agents: {_alice, _bob},
          );
          if (platform == TargetPlatform.iOS) {
            await _event(tester, states.last['session'] as String, 'query', {
              'value': 'Scout',
            });
            await _settleSearch(tester);
            final rows = states.last['users'] as List;
            expect(rows.map((row) => row['name']).toSet(), hasLength(2));
            for (final row in rows) {
              expect(row['name'], startsWith('Scout · '));
            }
            final session = states.last['session'] as String;
            await _event(tester, session, 'toggle', {'pubkey': _bob});
            final selected = (states.last['selected'] as List).single as Map;
            expect(selected['pubkey'], _bob);
            expect(selected['name'], (rows.last as Map)['name']);
            await _event(tester, session, 'submit');
          } else {
            await tester.enterText(
              find.byKey(const ValueKey('add-channel-members-search')),
              'Scout',
            );
            await _settleSearch(tester);
            final labels = tester
                .widgetList<Text>(find.byType(Text))
                .map((text) => text.data)
                .whereType<String>()
                .where((label) => label.startsWith('Scout · '))
                .toList();
            expect(labels.toSet(), hasLength(2));
            final semantics = tester.ensureSemantics();
            expect(
              find.bySemanticsLabel(RegExp(RegExp.escape(labels.last))),
              findsWidgets,
            );
            semantics.dispose();
            await tester.tap(find.text(labels.last));
            await tester.pumpAndSettle();
            await tester.tap(find.text('Add member'));
            await tester.pumpAndSettle();
          }
          expect(actions.calls, [
            [_bob],
          ]);
        } finally {
          await tester.pumpWidget(const SizedBox());
          TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
              .setMockMethodCallHandler(_bridge, null);
          debugDefaultTargetPlatformOverride = null;
        }
      },
    );
  }

  testWidgets(
    'iOS native search retains selections and retries only failed members',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final states = <Map<Object?, Object?>>[];
      final methods = <String>[];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(_bridge, (call) async {
            methods.add(call.method);
            if (call.method != 'dismiss') {
              states.add(Map<Object?, Object?>.from(call.arguments as Map));
            }
            return null;
          });
      try {
        final actions = _Actions()..failBob = true;
        await _pump(tester, actions);
        expect(methods.first, 'present');
        expect(find.byType(TextField), findsNothing);
        // No directory dump: people are found by name, as with `@`.
        expect(states.last['users'], isEmpty);
        expect(
          states.last['emptyQueryHint'],
          'Type a name to find people or agents.',
        );
        expect(states.last['clearQueryOnPick'], isNull);
        final session = states.last['session'] as String;
        await _event(tester, session, 'query', {'value': 'Al'});
        await _settleSearch(tester);
        await _event(tester, 'stale-session', 'toggle', {'pubkey': _alice});
        expect(states.last['selected'], isEmpty);
        await _event(tester, session, 'toggle', {'pubkey': _alice});
        await _event(tester, session, 'query', {'value': 'Bo'});
        expect(states.last['loading'], isTrue);
        await _settleSearch(tester);
        expect(states.last['loading'], isFalse);
        expect((states.last['users'] as List).map((row) => row['pubkey']), [
          _bob,
        ]);
        await _event(tester, session, 'toggle', {'pubkey': _bob});
        expect(states.last['selected'] as List, hasLength(2));
        // A member already in the channel is never offered.
        await _event(tester, session, 'query', {'value': 'Existing'});
        await _settleSearch(tester);
        expect(states.last['users'], isEmpty);
        await _event(tester, session, 'submit');
        expect(actions.calls, [
          [_alice, _bob],
        ]);
        expect((states.last['selected'] as List).map((row) => row['pubkey']), [
          _bob,
        ]);
        expect(states.last['error'], isNotNull);
        // The member just added is not offered again.
        await _event(tester, session, 'query', {'value': 'Al'});
        await _settleSearch(tester);
        expect(states.last['users'], isEmpty);
        actions.failBob = false;
        await _event(tester, session, 'submit');
        expect(actions.calls, [
          [_alice, _bob],
          [_bob],
        ]);
        expect(find.text('Open picker'), findsOneWidget);
        expect(methods.last, 'dismiss');
      } finally {
        await tester.pumpWidget(const SizedBox());
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(_bridge, null);
        debugDefaultTargetPlatformOverride = null;
      }
    },
  );

  testWidgets(
    'native picker renders member artwork and rejects stale avatar requests',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final states = <Map<Object?, Object?>>[];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(_bridge, (call) async {
            if (call.method != 'dismiss') {
              states.add(Map<Object?, Object?>.from(call.arguments as Map));
            }
            return null;
          });
      try {
        await _pump(tester, _Actions());
        final session = states.last['session'] as String;
        await _event(tester, session, 'query', {'value': 'Bo'});
        await _settleSearch(tester);
        final row = (states.last['users'] as List).single as Map;
        Future<Object?> request(String key, {String? requestSession}) async {
          final response = Completer<Object?>();
          await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
            _bridge.name,
            const StandardMethodCodec().encodeMethodCall(
              MethodCall('avatar', {
                'session': requestSession ?? session,
                'pubkey': _bob,
                'avatarKey': key,
              }),
            ),
            (data) => response.complete(
              const StandardMethodCodec().decodeEnvelope(data!),
            ),
          );
          return response.future;
        }

        final key = row['avatarKey'] as String;
        await tester.runAsync(() async {
          expect(await request('outdated-avatar'), isNull);
          expect(await request(key, requestSession: 'old-session'), isNull);
          final bytes = await request(key) as Uint8List;
          final codec = await ui.instantiateImageCodec(bytes);
          final frame = await codec.getNextFrame();
          final pixels = (await frame.image.toByteData())!;
          final center = (36 * frame.image.width + 36) * 4;
          expect(
            [for (var i = 0; i < 4; i++) pixels.getUint8(center + i)],
            [0, 0, 255, 255],
          );
          frame.image.dispose();
          codec.dispose();
          expect(await request(key), orderedEquals(bytes));
        });
        await _event(tester, session, 'toggle', {'pubkey': _bob});
        expect(
          ((states.last['selected'] as List).single as Map)['avatarKey'],
          key,
        );
        await _event(tester, session, 'closed');
      } finally {
        await tester.pumpWidget(const SizedBox());
        TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(_bridge, null);
        debugDefaultTargetPlatformOverride = null;
      }
    },
  );

  testWidgets('iOS dismiss sends no member writes', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    String? session;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(_bridge, (call) async {
          session = (call.arguments as Map)['session'] as String;
          return null;
        });
    try {
      final actions = _Actions();
      await _pump(tester, actions);
      await _event(tester, session!, 'query', {'value': 'Al'});
      await _settleSearch(tester);
      await _event(tester, session!, 'toggle', {'pubkey': _alice});
      await _event(tester, session!, 'closed');
      expect(actions.calls, isEmpty);
      expect(find.byType(AddChannelMembersSheet), findsNothing);
    } finally {
      await tester.pumpWidget(const SizedBox());
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(_bridge, null);
      debugDefaultTargetPlatformOverride = null;
    }
  });

  testWidgets('Android keeps the existing sheet and search field', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.android;
    try {
      await _pump(tester, _Actions());
      expect(find.text('Add members'), findsOneWidget);
      expect(
        find.byKey(const ValueKey('add-channel-members-search')),
        findsOneWidget,
      );
      expect(
        find.text('Type a name to find people or agents.'),
        findsOneWidget,
      );
      await tester.enterText(
        find.byKey(const ValueKey('add-channel-members-search')),
        'Al',
      );
      await _settleSearch(tester);
      expect(find.text('Alice'), findsOneWidget);
      // Picks show as chips, and the query stays for the next match.
      await tester.tap(find.text('Alice'));
      await tester.pumpAndSettle();
      expect(
        find.byKey(ValueKey('add-channel-member-selected-$_alice')),
        findsOneWidget,
      );
      expect(find.byKey(ValueKey('add-channel-member-$_alice')), findsNothing);
      expect(find.text('Al'), findsOneWidget);
    } finally {
      await tester.pumpWidget(const SizedBox());
      debugDefaultTargetPlatformOverride = null;
    }
  });
}
