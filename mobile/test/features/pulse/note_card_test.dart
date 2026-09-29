import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/features/pulse/note_card.dart';
import 'package:buzz/features/pulse/pulse_models.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _FakeUserCacheNotifier extends UserCacheNotifier {
  final Map<String, UserProfile> _users;

  _FakeUserCacheNotifier(this._users);

  @override
  Map<String, UserProfile> build() => _users;
}

void main() {
  testWidgets('constrains timestamp with agent and follow metadata', (
    tester,
  ) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(280, 600);
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });

    final note = UserNote(
      id: 'note-1',
      pubkey: 'alice',
      createdAt: DateTime.utc(2025, 9, 30, 12).millisecondsSinceEpoch ~/ 1000,
      content: 'A note',
      tags: const [],
    );

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          userCacheProvider.overrideWith(
            () => _FakeUserCacheNotifier({
              'alice': const UserProfile(
                pubkey: 'alice',
                displayName: 'A very long display name',
              ),
            }),
          ),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Builder(
            builder: (context) => MediaQuery(
              data: MediaQuery.of(
                context,
              ).copyWith(textScaler: const TextScaler.linear(2)),
              child: Scaffold(
                body: NoteCard(
                  note: note,
                  reaction: const PulseReactionState(
                    count: 0,
                    reactedByCurrentUser: false,
                  ),
                  isAgent: true,
                  canFollow: true,
                ),
              ),
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    final timestamp = tester.widget<Text>(find.text('Sep 30'));
    expect(timestamp.maxLines, 1);
    expect(timestamp.overflow, TextOverflow.ellipsis);
    expect(tester.takeException(), isNull);
  });

  testWidgets('gives the author unused timestamp width', (tester) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(320, 600);
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });
    const displayName = 'A moderately long Pulse author';
    final note = UserNote(
      id: 'note-2',
      pubkey: 'alice',
      createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 120,
      content: 'A note',
      tags: const [],
    );

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          userCacheProvider.overrideWith(
            () => _FakeUserCacheNotifier({
              'alice': const UserProfile(
                pubkey: 'alice',
                displayName: displayName,
              ),
            }),
          ),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Scaffold(
            body: NoteCard(
              note: note,
              reaction: const PulseReactionState(
                count: 0,
                reactedByCurrentUser: false,
              ),
              canFollow: true,
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(tester.getSize(find.text(displayName)).width, greaterThan(145));
    expect(find.text('2m'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets(
    'reply targets and author identities render npub keys, hex event ids, '
    'and keyed avatar initials',
    (tester) async {
      const b0b =
          'b0b0000000000000000000000000000000000000000000000000000000000000';
      const a11ce =
          'a11ce00000000000000000000000000000000000000000000000000000000000';
      const parentEventId =
          'feedbeef00000000000000000000000000000000000000000000000000000000';

      UserNote noteOf(String id, List<List<String>> tags) => UserNote(
        id: id,
        pubkey: b0b,
        createdAt: DateTime.utc(2025, 9, 30, 12).millisecondsSinceEpoch ~/ 1000,
        content: 'A reply',
        tags: tags,
      );

      final scenarios =
          <
            ({
              String id,
              List<List<String>> tags,
              Map<String, UserProfile> users,
              String authorLabel,
              String? replyLabel,
              int npubLabelCount,
              String avatarInitial,
            })
          >[
            // `p`-tagged parent: the reply target is a public key, so it renders
            // the compact npub — a second npub beside the unnamed author's own.
            (
              id: 'reply-parent-author',
              tags: [
                ['e', parentEventId, '', 'reply'],
                ['p', a11ce],
              ],
              users: const {},
              authorLabel: 'npub1kzc\u2026uyv8',
              replyLabel: 'Replying to npub15yw\u2026ccpw',
              npubLabelCount: 2,
              avatarInitial: 'B',
            ),
            // No `p` tag — the parent author is unknown, so the reply target
            // falls back to the parent event id, which is not a public key: it
            // keeps its hex truncation and stays out of the npub contract.
            (
              id: 'reply-parent-event',
              tags: [
                ['e', parentEventId, '', 'reply'],
              ],
              users: const {},
              authorLabel: 'npub1kzc\u2026uyv8',
              replyLabel: 'Replying to feedbeef\u2026',
              npubLabelCount: 1,
              avatarInitial: 'B',
            ),
            // Cached author: the authored name wins for both the label and the
            // avatar initial.
            (
              id: 'cached-named-author',
              tags: const [],
              users: {
                b0b: const UserProfile(pubkey: b0b, displayName: 'Carol'),
              },
              authorLabel: 'Carol',
              replyLabel: null,
              npubLabelCount: 0,
              avatarInitial: 'C',
            ),
            // Cached blank display names (empty or whitespace-only — relay
            // profiles can carry both) must fall back to the compact npub
            // for the label instead of rendering an empty author row.
            (
              id: 'cached-blank-name-author',
              tags: const [],
              users: {b0b: const UserProfile(pubkey: b0b, displayName: '')},
              authorLabel: 'npub1kzc\u2026uyv8',
              replyLabel: null,
              npubLabelCount: 1,
              avatarInitial: 'B',
            ),
            (
              id: 'cached-whitespace-name-author',
              tags: const [],
              users: {b0b: const UserProfile(pubkey: b0b, displayName: '   ')},
              authorLabel: 'npub1kzc\u2026uyv8',
              replyLabel: null,
              npubLabelCount: 1,
              avatarInitial: 'B',
            ),
          ];

      for (final scenario in scenarios) {
        // Each scenario remounts its own ProviderScope: Riverpod keeps an
        // updated scope's container, so an already-initialized user cache
        // would otherwise serve the first scenario's overrides.
        await tester.pumpWidget(
          KeyedSubtree(
            key: ValueKey(scenario.id),
            child: ProviderScope(
              overrides: [
                userCacheProvider.overrideWith(
                  () => _FakeUserCacheNotifier(scenario.users),
                ),
              ],
              child: MaterialApp(
                theme: AppTheme.light(),
                home: Scaffold(
                  body: NoteCard(
                    note: noteOf(scenario.id, scenario.tags),
                    reaction: const PulseReactionState(
                      count: 0,
                      reactedByCurrentUser: false,
                    ),
                  ),
                ),
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();

        expect(
          find.text(scenario.authorLabel),
          findsOneWidget,
          reason: scenario.id,
        );
        expect(
          find.textContaining('npub'),
          findsNWidgets(scenario.npubLabelCount),
          reason: scenario.id,
        );
        if (scenario.replyLabel != null) {
          expect(
            find.text(scenario.replyLabel!),
            findsOneWidget,
            reason: scenario.id,
          );
        }
        // Unnamed authors key the avatar initial to the hex public key
        // (never the `N` every npub label starts with); a cached profile
        // switches it to the authored-name initial.
        expect(
          _noteCardAvatarInitial(tester),
          scenario.avatarInitial,
          reason: scenario.id,
        );
        expect(tester.takeException(), isNull, reason: scenario.id);
      }
    },
  );
}

/// Avatar fallback initial of the single rendered [NoteCard] — asserts at
/// the production seam (the rendered card), not the model getter.
String _noteCardAvatarInitial(WidgetTester tester) {
  final card = find.byType(NoteCard);
  final avatar = find.descendant(of: card, matching: find.byType(AvatarImage));
  final initial = tester.widget<Text>(
    find.descendant(of: avatar, matching: find.byType(Text)),
  );
  return initial.data!;
}
