import 'package:buzz/features/pulse/pulse_models.dart';
import 'package:buzz/features/pulse/pulse_page.dart';
import 'package:buzz/features/pulse/pulse_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _UserCache extends UserCacheNotifier {
  _UserCache(this._users);

  final Map<String, UserProfile> _users;

  @override
  Map<String, UserProfile> build() => _users;

  @override
  UserProfile? get(String pubkey) => _users[pubkey.toLowerCase()];

  @override
  Future<bool> preload(List<String> pubkeys) async => true;
}

void main() {
  testWidgets('iOS tab changes reset collapsed title and timeline together', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    const channel = MethodChannel('buzz/ios_navigation_bar/987');
    final calls = <MethodCall>[];
    tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(channel, (
      call,
    ) async {
      calls.add(call);
      return null;
    });
    addTearDown(
      () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        channel,
        null,
      ),
    );
    final author = '1' * 64;
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          userCacheProvider.overrideWith(
            () => _UserCache({
              author: UserProfile(pubkey: author, displayName: 'Alice'),
            }),
          ),
          myPubkeyProvider.overrideWithValue(null),
          agentPubkeysProvider.overrideWith((ref) async => const []),
          globalNotesProvider.overrideWith(
            (ref) async => List.generate(
              30,
              (i) => UserNote(
                id: 'note-$i',
                pubkey: author,
                createdAt: 1759233600,
                content: 'Timeline note $i',
                tags: const [],
              ),
            ),
          ),
          likedNotesProvider.overrideWith((ref) async => const []),
          noteReactionsProvider.overrideWith((ref, key) async => const {}),
        ],
        child: MaterialApp(theme: AppTheme.light(), home: const PulsePage()),
      ),
    );
    await tester.pumpAndSettle();
    void bindNative() => tester
        .widget<UiKitView>(find.byType(UiKitView))
        .onPlatformViewCreated!(987);
    bindNative();
    await tester.pump();
    final expandedHeight = tester.getSize(find.byType(UiKitView)).height;
    final expandedTabsTop = tester.getTopLeft(find.text('Everyone')).dy;
    final row = find.text('Timeline note 1');
    final gesture = await tester.startGesture(
      tester.getCenter(find.byType(ListView).last),
    );
    await gesture.moveBy(const Offset(0, -20));
    await tester.pump();
    for (var i = 0; i < 5; i++) {
      final before = tester.getTopLeft(row).dy;
      await gesture.moveBy(const Offset(0, -20));
      await tester.pump();
      expect(before - tester.getTopLeft(row).dy, closeTo(20, 1));
    }
    await gesture.up();
    await tester.pumpAndSettle();
    await tester.drag(find.byType(ListView).last, const Offset(0, -400));
    await tester.pumpAndSettle();
    expect(calls.where((c) => c.method == 'scroll').last.arguments, 52);
    expect(
      tester.getSize(find.byType(UiKitView)).height,
      lessThan(expandedHeight),
    );
    expect(tester.getTopLeft(find.text('Everyone')).dy, expandedTabsTop - 52);
    await tester.tap(find.text('Liked'));
    await tester.pumpAndSettle();
    bindNative();
    await tester.pump();
    expect(calls.where((c) => c.method == 'scroll').last.arguments, 0);
    expect(tester.getSize(find.byType(UiKitView)).height, expandedHeight);
    expect(tester.getTopLeft(find.text('Everyone')).dy, expandedTabsTop);
    await tester.tap(find.text('Everyone'));
    await tester.pumpAndSettle();
    bindNative();
    await tester.pump();
    expect(calls.where((c) => c.method == 'scroll').last.arguments, 0);
    expect(find.text('Timeline note 0'), findsOneWidget);
    expect(tester.getTopLeft(find.text('Everyone')).dy, expandedTabsTop);
    await tester.pumpWidget(const SizedBox());
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('same-name reply targets on different notes are told apart', (
    tester,
  ) async {
    // Two authors reply to two different people who are not authors on the
    // timeline but share the name Scout.
    final alice = '1' * 64, bob = '2' * 64;
    final scoutA = 'a' * 64, scoutB = 'b' * 64;
    final users = {
      alice: UserProfile(pubkey: alice, displayName: 'Alice'),
      bob: UserProfile(pubkey: bob, displayName: 'Bob'),
      scoutA: UserProfile(pubkey: scoutA, displayName: 'Scout'),
      scoutB: UserProfile(pubkey: scoutB, displayName: 'Scout'),
    };
    final createdAt =
        DateTime.utc(2025, 9, 30, 12).millisecondsSinceEpoch ~/ 1000;
    UserNote reply(String id, String author, String target) => UserNote(
      id: id,
      pubkey: author,
      createdAt: createdAt,
      content: 'A reply',
      tags: [
        ['e', 'parent-$id', '', 'reply'],
        ['p', target],
      ],
    );

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          userCacheProvider.overrideWith(() => _UserCache(users)),
          myPubkeyProvider.overrideWithValue(null),
          agentPubkeysProvider.overrideWith((ref) async => const []),
          globalNotesProvider.overrideWith(
            (ref) async => [
              reply('note-1', alice, scoutA),
              reply('note-2', bob, scoutB),
            ],
          ),
          noteReactionsProvider.overrideWith((ref, key) async => const {}),
        ],
        child: MaterialApp(theme: AppTheme.light(), home: const PulsePage()),
      ),
    );
    await tester.pumpAndSettle();

    final replyLabels = tester
        .widgetList<Text>(find.textContaining('Replying to '))
        .map((text) => text.data)
        .toList();
    expect(replyLabels, hasLength(2));
    expect(replyLabels.toSet(), hasLength(2));
    expect(replyLabels, isNot(contains('Replying to Scout')));
  });
}
