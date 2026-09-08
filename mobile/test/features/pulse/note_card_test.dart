import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/features/pulse/note_card.dart';
import 'package:buzz/features/pulse/pulse_models.dart';
import 'package:buzz/shared/theme/theme.dart';
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

  testWidgets('replying-to shows the parent author as a compact npub', (
    tester,
  ) async {
    const authorPubkey =
        'b0b0000000000000000000000000000000000000000000000000000000000000';
    const parentAuthorPubkey =
        'a11ce00000000000000000000000000000000000000000000000000000000000';
    final note = UserNote(
      id: 'note-reply-author',
      pubkey: authorPubkey,
      createdAt: DateTime.utc(2025, 9, 30, 12).millisecondsSinceEpoch ~/ 1000,
      content: 'A reply',
      tags: const [
        ['e', 'parent-event-1', '', 'reply'],
        ['p', parentAuthorPubkey],
      ],
    );

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          userCacheProvider.overrideWith(
            () => _FakeUserCacheNotifier(const {}),
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
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    // The known parent author renders as a compact npub.
    expect(find.text('Replying to npub15yw\u2026ccpw'), findsOneWidget);
    // The note author itself falls back to the compact npub label.
    expect(find.text('npub1kzc\u2026uyv8'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('replying-to keeps event ids hex-truncated, never npub', (
    tester,
  ) async {
    const authorPubkey =
        'b0b0000000000000000000000000000000000000000000000000000000000000';
    const parentEventId =
        'feedbeef00000000000000000000000000000000000000000000000000000000';
    final note = UserNote(
      id: 'note-reply-event',
      pubkey: authorPubkey,
      createdAt: DateTime.utc(2025, 9, 30, 12).millisecondsSinceEpoch ~/ 1000,
      content: 'A reply',
      tags: const [
        // No `p` tag — the parent author is unknown, so the reply target
        // falls back to the parent event id, which is not a public key.
        ['e', parentEventId, '', 'reply'],
      ],
    );

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          userCacheProvider.overrideWith(
            () => _FakeUserCacheNotifier(const {}),
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
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    // Event ids stay in their hex truncation — outside the npub contract.
    expect(find.text('Replying to feedbeef\u2026'), findsOneWidget);
    expect(find.textContaining('npub'), findsNWidgets(1)); // only the author
    expect(tester.takeException(), isNull);
  });
}
