import 'package:buzz/features/channels/reaction_row.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/emoji/emoji_burst.dart';
import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:buzz/features/profile/user_cache_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/widget_helpers.dart';

const _fire = '\u{1F525}';

/// 👀 — a real reaction that is deliberately not in the positive-burst set.
const _eyes = '\u{1F440}';

TimelineReaction _reaction({
  String emoji = _fire,
  int count = 1,
  bool reactedByCurrentUser = false,
  List<String> userPubkeys = const ['alice'],
}) => TimelineReaction(
  emoji: emoji,
  count: count,
  reactedByCurrentUser: reactedByCurrentUser,
  userPubkeys: userPubkeys,
);

final _dataset = EmojiDataset(
  categories: const [],
  all: const [
    EmojiEntry(
      id: 'fire',
      name: 'Fire',
      keywords: [],
      native: _fire,
      categoryId: 'nature',
    ),
  ],
  nativeToShortcode: const {_fire: ':fire:'},
);

const _messageId = 'msg-1';

/// Pumps the row and hands back the enclosing container, so tests can arm a
/// pending burst or inspect the particle controller.
///
/// Re-pumping keeps the same `ProviderScope` state, which is what lets a test
/// swap the reactions list — standing in for the relay echo — without losing
/// the pending burst that was armed before it.
Future<ProviderContainer> _pumpRow(
  WidgetTester tester, {
  required List<TimelineReaction> reactions,
  void Function(String emoji)? onToggle,
  bool showAddButton = false,
  VoidCallback? onAddReaction,
  String messageId = _messageId,
}) async {
  await tester.pumpWidget(
    WidgetHelpers.testable(
      overrides: [
        emojiDatasetOrEmptyProvider.overrideWithValue(_dataset),
        userCacheProvider.overrideWith(
          () => _FakeUserCacheNotifier({
            'alice': const UserProfile(pubkey: 'alice', displayName: 'Alice'),
          }),
        ),
      ],
      child: ReactionRow(
        messageId: messageId,
        reactions: reactions,
        onToggle: onToggle ?? (_) {},
        showAddButton: showAddButton,
        onAddReaction: onAddReaction,
      ),
    ),
  );
  await tester.pumpAndSettle();
  return ProviderScope.containerOf(tester.element(find.byType(ReactionRow)));
}

class _FakeUserCacheNotifier extends UserCacheNotifier {
  final Map<String, UserProfile> _profiles;

  _FakeUserCacheNotifier(this._profiles);

  @override
  Map<String, UserProfile> build() => _profiles;

  @override
  Future<void> preload(Iterable<String> pubkeys) async {}
}

void main() {
  group('ReactionRow', () {
    testWidgets('shows the count even at one, matching desktop', (
      tester,
    ) async {
      await _pumpRow(tester, reactions: [_reaction()]);

      expect(
        find.byKey(const ValueKey('reaction-pill-$_fire')),
        findsOneWidget,
      );
      expect(find.text('1'), findsOneWidget);
    });

    testWidgets('collapses entirely when there is nothing to show', (
      tester,
    ) async {
      await _pumpRow(tester, reactions: const []);

      expect(find.byType(Wrap), findsNothing);
      expect(find.byKey(const ValueKey('add-reaction-pill')), findsNothing);
    });

    testWidgets('renders the + pill on an empty row when asked', (
      tester,
    ) async {
      var taps = 0;
      await _pumpRow(
        tester,
        reactions: const [],
        showAddButton: true,
        onAddReaction: () => taps++,
      );

      final addPill = find.byKey(const ValueKey('add-reaction-pill'));
      expect(addPill, findsOneWidget);
      await tester.tap(addPill);
      expect(taps, 1);
    });

    testWidgets('+ pill trails the existing reactions', (tester) async {
      await _pumpRow(
        tester,
        reactions: [
          _reaction(),
          _reaction(emoji: '\u{1F44D}', count: 3),
        ],
        showAddButton: true,
        onAddReaction: () {},
      );

      final firstPill = tester.getRect(
        find.byKey(const ValueKey('reaction-pill-$_fire')),
      );
      final lastPill = tester.getRect(
        find.byKey(const ValueKey('reaction-pill-\u{1F44D}')),
      );
      final addPill = tester.getRect(
        find.byKey(const ValueKey('add-reaction-pill')),
      );

      // One row, in order, with the + last.
      expect(lastPill.left, greaterThan(firstPill.left));
      expect(addPill.left, greaterThan(lastPill.left));
      expect(addPill.top, firstPill.top);

      // Pills hug their content. A `Container.alignment` here would silently
      // stretch each one to the full row width and stack them vertically.
      expect(firstPill.height, 28);
      expect(firstPill.width, greaterThanOrEqualTo(48));
      expect(firstPill.width, lessThan(120));
      expect(addPill.width, 40);
    });

    testWidgets('the + pill stays hidden without a handler', (tester) async {
      await _pumpRow(tester, reactions: [_reaction()], showAddButton: true);

      expect(find.byKey(const ValueKey('add-reaction-pill')), findsNothing);
    });

    testWidgets('tapping a pill toggles that emoji', (tester) async {
      final toggled = <String>[];
      await _pumpRow(tester, reactions: [_reaction()], onToggle: toggled.add);

      await tester.tap(find.byKey(const ValueKey('reaction-pill-$_fire')));
      expect(toggled, [_fire]);
    });

    testWidgets('long-press opens the detail sheet named from the dataset', (
      tester,
    ) async {
      await _pumpRow(
        tester,
        reactions: [
          _reaction(count: 1, userPubkeys: const ['alice']),
        ],
      );

      await tester.longPress(
        find.byKey(const ValueKey('reaction-pill-$_fire')),
      );
      await tester.pumpAndSettle();

      expect(find.text(':fire:'), findsOneWidget);
      expect(find.text('Alice'), findsOneWidget);
    });
  });

  group('reaction bursts', () {
    testWidgets('adding a positive reaction bursts from the pill', (
      tester,
    ) async {
      final container = await _pumpRow(tester, reactions: [_reaction()]);
      final controller = container.read(emojiBurstControllerProvider);
      expect(controller.hasParticles, isFalse);

      await tester.tap(find.byKey(const ValueKey('reaction-pill-$_fire')));
      await tester.pump();

      expect(controller.hasParticles, isTrue);
    });

    testWidgets('a non-positive reaction does not burst', (tester) async {
      final container = await _pumpRow(
        tester,
        reactions: [_reaction(emoji: _eyes)],
      );
      final controller = container.read(emojiBurstControllerProvider);

      await tester.tap(find.byKey(const ValueKey('reaction-pill-$_eyes')));
      await tester.pump();

      expect(controller.hasParticles, isFalse);
    });

    testWidgets('removing your own reaction does not burst', (tester) async {
      final container = await _pumpRow(
        tester,
        reactions: [_reaction(reactedByCurrentUser: true)],
      );
      final controller = container.read(emojiBurstControllerProvider);

      await tester.tap(find.byKey(const ValueKey('reaction-pill-$_fire')));
      await tester.pump();

      expect(controller.hasParticles, isFalse);
    });

    testWidgets('a pending burst fires when the new pill arrives', (
      tester,
    ) async {
      // Reacting from the quick row or the picker arms a burst, then the pill
      // only appears once the relay echoes the reaction back.
      final container = await _pumpRow(tester, reactions: const []);
      final controller = container.read(emojiBurstControllerProvider);
      container
          .read(pendingReactionBurstProvider.notifier)
          .arm(_messageId, _fire);

      await _pumpRow(
        tester,
        reactions: [_reaction(reactedByCurrentUser: true)],
      );
      await tester.pump();

      expect(controller.hasParticles, isTrue);
      // Claimed, so a second row showing the same message can't double-burst.
      expect(container.read(pendingReactionBurstProvider), isNull);
    });

    testWidgets('a pending burst waits for our own reaction, not anyone\'s', (
      tester,
    ) async {
      final container = await _pumpRow(tester, reactions: const []);
      final controller = container.read(emojiBurstControllerProvider);
      container
          .read(pendingReactionBurstProvider.notifier)
          .arm(_messageId, _fire);

      // Someone else's reaction lands first.
      await _pumpRow(tester, reactions: [_reaction()]);
      await tester.pump();

      expect(controller.hasParticles, isFalse);
      expect(container.read(pendingReactionBurstProvider), isNotNull);
    });

    testWidgets('a pending burst armed for another message is left alone', (
      tester,
    ) async {
      final container = await _pumpRow(tester, reactions: const []);
      final controller = container.read(emojiBurstControllerProvider);
      container
          .read(pendingReactionBurstProvider.notifier)
          .arm('some-other-message', _fire);

      await _pumpRow(
        tester,
        reactions: [_reaction(reactedByCurrentUser: true)],
      );
      await tester.pump();

      expect(controller.hasParticles, isFalse);
      expect(container.read(pendingReactionBurstProvider), isNotNull);
    });

    testWidgets(
      'a pill under a pushed route leaves the burst for the top one',
      (tester) async {
        // The channel timeline stays mounted under a pushed thread page and
        // rebuilds first, so without a route check it claimed the burst and the
        // user saw nothing until they popped back.
        final navigatorKey = GlobalKey<NavigatorState>();
        late ProviderContainer container;

        await tester.pumpWidget(
          ProviderScope(
            overrides: [
              emojiDatasetOrEmptyProvider.overrideWithValue(_dataset),
              userCacheProvider.overrideWith(
                () => _FakeUserCacheNotifier(const {}),
              ),
            ],
            child: MaterialApp(
              navigatorKey: navigatorKey,
              home: Scaffold(
                body: ReactionRow(
                  messageId: _messageId,
                  reactions: [_reaction(reactedByCurrentUser: true)],
                  onToggle: (_) {},
                ),
              ),
            ),
          ),
        );
        await tester.pumpAndSettle();
        container = ProviderScope.containerOf(
          tester.element(find.byType(ReactionRow)),
        );
        final controller = container.read(emojiBurstControllerProvider);

        // Push a route over it, then arm and let the covered pill rebuild.
        navigatorKey.currentState!.push(
          MaterialPageRoute<void>(
            builder: (_) => const Scaffold(body: Text('thread')),
          ),
        );
        await tester.pumpAndSettle();
        container
            .read(pendingReactionBurstProvider.notifier)
            .arm(_messageId, _fire);
        await tester.pump();

        expect(controller.hasParticles, isFalse);
        // Still armed, so the pill on the visible route can claim it.
        expect(container.read(pendingReactionBurstProvider), isNotNull);
      },
    );

    testWidgets('reduced motion suppresses the burst', (tester) async {
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            emojiDatasetOrEmptyProvider.overrideWithValue(_dataset),
            userCacheProvider.overrideWith(
              () => _FakeUserCacheNotifier(const {}),
            ),
          ],
          child: MediaQuery(
            data: const MediaQueryData(disableAnimations: true),
            child: MaterialApp(
              home: Scaffold(
                body: ReactionRow(
                  messageId: _messageId,
                  reactions: [_reaction()],
                  onToggle: (_) {},
                ),
              ),
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      final container = ProviderScope.containerOf(
        tester.element(find.byType(ReactionRow)),
      );
      final controller = container.read(emojiBurstControllerProvider);

      await tester.tap(find.byKey(const ValueKey('reaction-pill-$_fire')));
      await tester.pump();

      expect(controller.hasParticles, isFalse);
    });
  });
}
