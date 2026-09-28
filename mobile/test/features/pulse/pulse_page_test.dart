import 'package:buzz/features/pulse/pulse_models.dart';
import 'package:buzz/features/pulse/pulse_page.dart';
import 'package:buzz/features/pulse/pulse_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
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
