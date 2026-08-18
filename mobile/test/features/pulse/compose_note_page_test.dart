import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/profile/user_cache_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/features/pulse/compose_note_page.dart';
import 'package:buzz/features/pulse/pulse_models.dart';
import 'package:buzz/shared/theme/theme.dart';

class _FakeUserCacheNotifier extends UserCacheNotifier {
  final Map<String, UserProfile> _users;
  _FakeUserCacheNotifier(this._users);

  @override
  Map<String, UserProfile> build() => _users;
}

void main() {
  final replyNote = UserNote(
    id: 'note1',
    pubkey: 'alice_pk',
    createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 120,
    content: 'The original note being replied to',
    tags: const [],
  );

  Widget buildTestable(
    Widget home, {
    TextScaler textScaler = TextScaler.noScaling,
    String displayName = 'Alice',
  }) {
    return ProviderScope(
      overrides: [
        userCacheProvider.overrideWith(
          () => _FakeUserCacheNotifier({
            'alice_pk': UserProfile(
              pubkey: 'alice_pk',
              displayName: displayName,
            ),
          }),
        ),
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        home: Builder(
          builder: (context) => MediaQuery(
            data: MediaQuery.of(context).copyWith(textScaler: textScaler),
            child: home,
          ),
        ),
      ),
    );
  }

  testWidgets('reply mode shows a rich preview of the replied-to note', (
    tester,
  ) async {
    await tester.pumpWidget(buildTestable(ComposeNotePage(replyTo: replyNote)));
    await tester.pump();

    expect(find.text('Replying to Alice'), findsOneWidget);
    expect(find.text('Alice'), findsOneWidget); // author name in the row
    expect(find.textContaining('original note being replied to'), findsWidgets);
    expect(find.text('Reply'), findsOneWidget); // action button label
  });

  testWidgets('reply preview constrains its timestamp at large text sizes', (
    tester,
  ) async {
    final oldReplyNote = UserNote(
      id: 'old-note',
      pubkey: 'alice_pk',
      createdAt: DateTime.utc(2025, 9, 30, 12).millisecondsSinceEpoch ~/ 1000,
      content: 'An older note',
      tags: const [],
    );

    await tester.pumpWidget(
      buildTestable(
        ComposeNotePage(replyTo: oldReplyNote),
        textScaler: const TextScaler.linear(2),
      ),
    );
    await tester.pump();

    final timestamp = tester.widget<Text>(find.text('Sep 30'));
    expect(timestamp.maxLines, 1);
    expect(timestamp.overflow, TextOverflow.ellipsis);
    expect(tester.takeException(), isNull);
  });

  testWidgets('gives the reply author unused timestamp width', (tester) async {
    tester.view.devicePixelRatio = 1;
    tester.view.physicalSize = const Size(320, 600);
    addTearDown(() {
      tester.view.resetPhysicalSize();
      tester.view.resetDevicePixelRatio();
    });
    const displayName = 'A moderately long Pulse reply author';

    await tester.pumpWidget(
      buildTestable(
        ComposeNotePage(replyTo: replyNote),
        displayName: displayName,
      ),
    );
    await tester.pump();

    expect(tester.getSize(find.text(displayName)).width, greaterThan(150));
    expect(find.text('2m'), findsOneWidget);
    expect(tester.takeException(), isNull);
  });

  testWidgets('new-note mode shows no reply preview', (tester) async {
    await tester.pumpWidget(buildTestable(const ComposeNotePage()));
    await tester.pump();

    expect(find.textContaining('Replying to'), findsNothing);
    expect(find.text('Post'), findsOneWidget);
  });

  testWidgets('reply preview stays compact for a short note', (tester) async {
    await tester.pumpWidget(buildTestable(ComposeNotePage(replyTo: replyNote)));
    await tester.pump();

    // The preview content sits just above the divider; for a one-line note
    // the gap between the content text and the divider must be small (this
    // guards the earlier "preview row way too tall" regression).
    final contentBottom = tester
        .getRect(find.text('The original note being replied to'))
        .bottom;
    final divider = find.byType(Divider);
    expect(divider, findsOneWidget);
    final dividerTop = tester.getRect(divider).top;
    expect(
      dividerTop - contentBottom,
      lessThan(24),
      reason: 'reply preview should hug its content, not reserve tall space',
    );
  });

  testWidgets('reply preview with an image note clips to a bounded height', (
    tester,
  ) async {
    final imageNote = UserNote(
      id: 'note2',
      pubkey: 'alice_pk',
      createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000 - 60,
      content: '#buzz\n![image](https://example.com/big.png)',
      tags: const [
        [
          'imeta',
          'url https://example.com/big.png',
          'm image/png',
          'dim 800x1600',
        ],
      ],
    );
    await tester.pumpWidget(buildTestable(ComposeNotePage(replyTo: imageNote)));
    await tester.pump();

    // Renders the rich content (not the raw "![image](...)" markdown).
    expect(find.textContaining('![image]'), findsNothing);
    // The whole reply context stays bounded: divider sits within a screen of
    // the "Replying to" label (guards a tall image blowing up the page).
    final labelTop = tester.getRect(find.text('Replying to Alice')).top;
    final dividerTop = tester.getRect(find.byType(Divider)).top;
    expect(dividerTop - labelTop, lessThan(220));
  });
}
