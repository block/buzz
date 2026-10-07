import 'package:buzz/shared/emoji/emoji_avatar.dart';
import 'dart:async';
import 'package:buzz/features/channels/channel_identity_names_provider.dart';
import 'package:buzz/shared/identity_names/identity_names.dart';
import 'package:buzz/features/channels/message_long_press_region.dart';
import 'package:buzz/features/channels/reaction_row.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/shared/emoji/emoji_burst.dart';
import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:buzz/shared/widgets/native_message_presentation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/widget_helpers.dart';

const _alice =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';

const _fire = '\u{1F525}';

/// 👀 — a real reaction that is deliberately not in the positive-burst set.
const _eyes = '\u{1F440}';

TimelineReaction _reaction({
  String emoji = _fire,
  int count = 1,
  bool reactedByCurrentUser = false,
  List<String> userPubkeys = const [_alice],
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
  VoidCallback? onMessageLongPress,
}) async {
  await tester.pumpWidget(
    WidgetHelpers.testable(
      overrides: [
        emojiDatasetOrEmptyProvider.overrideWithValue(_dataset),
        userCacheProvider.overrideWith(
          () => _FakeUserCacheNotifier({
            _alice: const UserProfile(pubkey: _alice, displayName: 'Alice'),
          }),
        ),
      ],
      child: MessageLongPressInkWell(
        onLongPress: (_) => onMessageLongPress?.call(),
        child: ReactionRow(
          messageId: messageId,
          channelId: 'channel',
          reactions: reactions,
          onToggle: onToggle ?? (_) {},
          showAddButton: showAddButton,
          onAddReaction: onAddReaction,
        ),
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

  void replace(Map<String, UserProfile> profiles) => state = profiles;

  @override
  Future<bool> preload(List<String> pubkeys) async => true;
}

void main() {
  testWidgets(
    'native reactor labels retain channel qualifications and live updates',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      final bot = 'b' * 64;
      final roster = 'c' * 64;
      var directoryName = 'Honey';
      final calls = <Map<Object?, Object?>>[];
      final mergedProfiles = <Object?, Object?>{};
      final completion = Completer<Map<String, Object?>>();
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        (call) async {
          if (call.method == 'reactions' || call.method == 'updateProfiles') {
            final args = call.arguments as Map<Object?, Object?>;
            mergedProfiles.addAll(args['profiles'] as Map);
            calls.add({...args, 'profiles': Map.of(mergedProfiles)});
          }
          if (call.method == 'reactions') return completion.future;
          return <String, Object?>{};
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          NativeMessagePresentation.channel,
          null,
        ),
      );
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            emojiDatasetOrEmptyProvider.overrideWithValue(_dataset),
            userCacheProvider.overrideWith(
              () => _FakeUserCacheNotifier({
                _alice: const UserProfile(pubkey: _alice, displayName: 'Honey'),
              }),
            ),
            channelIdentityNamesProvider('channel').overrideWith(
              (ref) =>
                  IdentityNameSources(
                    profiles: ref.watch(userCacheProvider),
                    agentDisplayNames: {bot: directoryName},
                  ).scope(
                    [_alice, bot, roster],
                    agentPubkeys: {bot},
                    fallbackNames: {roster: 'Roster name'},
                  ),
            ),
          ],
          child: ReactionRow(
            messageId: _messageId,
            channelId: 'channel',
            reactions: [
              _reaction(userPubkeys: [_alice, bot, roster]),
            ],
            onToggle: (_) {},
          ),
        ),
      );
      await tester.pumpAndSettle();
      final container = ProviderScope.containerOf(
        tester.element(find.byType(ReactionRow)),
      );
      await tester.longPress(
        find.byKey(const ValueKey('reaction-pill-$_fire')),
      );
      await tester.pumpAndSettle();
      List<Object?> labels(Map<Object?, Object?> payload) {
        final profiles = payload['profiles'] as Map;
        return [
          for (final key in [_alice, bot, roster])
            (profiles[key] as Map)['name'],
        ];
      }

      expect(labels(calls.first), ['Honey', 'Honey (agent)', 'Roster name']);
      // Directory/roster changes must propagate without a user-cache event.
      directoryName = 'Helper';
      container.invalidate(channelIdentityNamesProvider('channel'));
      await tester.pumpAndSettle();
      expect(labels(calls.last), ['Honey', 'Helper', 'Roster name']);
      (container.read(userCacheProvider.notifier) as _FakeUserCacheNotifier)
          .replace({
            _alice: const UserProfile(pubkey: _alice, displayName: 'Helper'),
          });
      await tester.pumpAndSettle();
      expect(labels(calls.last), ['Helper', 'Helper (agent)', 'Roster name']);
      expect(calls.map((c) => c['requestId']).toSet(), hasLength(1));
      completion.complete(<String, Object?>{});
      await tester.pumpAndSettle();
      final count = calls.length;
      directoryName = 'Closed';
      container.invalidate(channelIdentityNamesProvider('channel'));
      await tester.pumpAndSettle();
      expect(calls, hasLength(count));
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets(
    'native hydration sends bounded diffs and preserves emoji avatars',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final pubkeys = [
        for (var i = 1; i <= 2500; i++) i.toRadixString(16).padLeft(64, '0'),
      ];
      final completion = Completer<Map<String, Object?>>();
      final updates = <Map>[];
      Map? initial;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        (call) async {
          final args = call.arguments as Map;
          if (call.method == 'reactions') {
            initial = args['profiles'] as Map;
            return completion.future;
          }
          if (call.method == 'updateProfiles') {
            updates.add(args['profiles'] as Map);
          }
          return <String, Object?>{};
        },
      );
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            emojiDatasetOrEmptyProvider.overrideWithValue(_dataset),
            userCacheProvider.overrideWith(() => _FakeUserCacheNotifier({})),
            channelIdentityNamesProvider('channel').overrideWith(
              (ref) => IdentityNameSources(
                profiles: ref.watch(userCacheProvider),
              ).scope(pubkeys),
            ),
          ],
          child: ReactionRow(
            messageId: _messageId,
            channelId: 'channel',
            reactions: [_reaction(userPubkeys: pubkeys)],
            onToggle: (_) {},
          ),
        ),
      );
      await tester.pumpAndSettle();
      final container = ProviderScope.containerOf(
        tester.element(find.byType(ReactionRow)),
      );
      await tester.longPress(
        find.byKey(const ValueKey('reaction-pill-$_fire')),
      );
      await tester.pumpAndSettle();
      expect(initial, hasLength(2500));
      final cache = <String, UserProfile>{};
      for (var start = 0; start < pubkeys.length; start += 1000) {
        for (final key in pubkeys.skip(start).take(1000)) {
          cache[key] = UserProfile(
            pubkey: key,
            displayName: 'Person $key',
            avatarUrl: key == pubkeys.first
                ? emojiAvatarDataUrl('😊', 0xFFFF6B9A)
                : null,
          );
        }
        (container.read(userCacheProvider.notifier) as _FakeUserCacheNotifier)
            .replace(Map.of(cache));
        await tester.pumpAndSettle();
      }
      expect(
        updates.length,
        10,
      ); // 4 + 4 + 2 chunks, rather than full-set payloads.
      expect(updates.every((p) => p.length <= 256), isTrue);
      expect(updates.fold<int>(0, (sum, p) => sum + p.length), 2500);
      final avatar = updates.first[pubkeys.first] as Map;
      expect(avatar['avatarEmoji'], '😊');
      expect(avatar['avatarColor'], 0xFFFF6B9A);
      expect(avatar.containsKey('url'), isFalse);
      cache['f' * 64] = UserProfile(pubkey: 'f' * 64, displayName: 'Unrelated');
      (container.read(userCacheProvider.notifier) as _FakeUserCacheNotifier)
          .replace(Map.of(cache));
      await tester.pumpAndSettle();
      expect(updates, hasLength(10));
      completion.complete(<String, Object?>{});
      await tester.pumpAndSettle();
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        null,
      );
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets(
    'holding a pill opens native membership with the selected emoji without toggling',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      Map<Object?, Object?>? payload;
      tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
        NativeMessagePresentation.channel,
        (call) async {
          if (call.method == 'reactions') {
            payload = call.arguments as Map<Object?, Object?>;
          }
          return <String, Object?>{};
        },
      );
      addTearDown(
        () => tester.binding.defaultBinaryMessenger.setMockMethodCallHandler(
          NativeMessagePresentation.channel,
          null,
        ),
      );
      var toggles = 0;
      var messageHolds = 0;
      await _pumpRow(
        tester,
        reactions: [
          _reaction(),
          _reaction(emoji: _eyes),
        ],
        onToggle: (_) => toggles++,
        onMessageLongPress: () => messageHolds++,
      );
      await tester.longPress(
        find.byKey(const ValueKey('reaction-pill-$_eyes')),
      );
      await tester.pumpAndSettle();
      expect(toggles, 0);
      expect(messageHolds, 0);
      expect(payload!['initialEmoji'], _eyes);
      expect((payload!['reactions'] as List).length, 2);
      final profiles = payload!['profiles'] as Map;
      expect((profiles[_alice] as Map)['name'], 'Alice');
      expect(find.byType(BottomSheet), findsNothing);
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets(
    'Android reaction counts open attribution without toggling or opening message actions',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.android;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      var toggles = 0;
      var messageHolds = 0;
      await _pumpRow(
        tester,
        reactions: [
          _reaction(reactedByCurrentUser: true),
          _reaction(emoji: _eyes),
        ],
        onToggle: (_) => toggles++,
        onMessageLongPress: () => messageHolds++,
      );
      final pill = find.byKey(const ValueKey('reaction-pill-$_eyes'));
      await tester.longPress(
        find.descendant(of: pill, matching: find.text('1')),
      );
      await tester.pumpAndSettle();
      expect(find.text('Reactions'), findsNothing);
      expect(find.text('All 2'), findsOneWidget);
      final controller = DefaultTabController.of(
        tester.element(find.byType(TabBar)),
      );
      expect(controller.index, 2);
      expect(
        find.byKey(ValueKey('reactor-$_alice-$_eyes')).hitTestable(),
        findsOneWidget,
      );
      expect(
        find.byKey(ValueKey('reactor-$_alice-$_fire')).hitTestable(),
        findsNothing,
      );
      await tester.tap(find.byKey(const ValueKey('reaction-filter-all')));
      await tester.pumpAndSettle();
      expect(controller.index, 0);
      expect(
        find.byKey(ValueKey('reactor-$_alice-$_eyes')).hitTestable(),
        findsOneWidget,
      );
      expect(
        find.byKey(ValueKey('reactor-$_alice-$_fire')).hitTestable(),
        findsOneWidget,
      );
      await tester.tap(find.byKey(const ValueKey('reaction-filter-$_fire')));
      await tester.pumpAndSettle();
      expect(controller.index, 1);
      expect(
        find.byKey(ValueKey('reactor-$_alice-$_eyes')).hitTestable(),
        findsNothing,
      );
      expect(
        find.byKey(ValueKey('reactor-$_alice-$_fire')).hitTestable(),
        findsOneWidget,
      );
      await tester.drag(find.byType(TabBarView), const Offset(-500, 0));
      await tester.pumpAndSettle();
      expect(controller.index, 2);
      expect(
        find.byKey(ValueKey('reactor-$_alice-$_eyes')).hitTestable(),
        findsOneWidget,
      );
      expect(
        find.byKey(ValueKey('reactor-$_alice-$_fire')).hitTestable(),
        findsNothing,
      );
      expect(toggles, 0);
      expect(messageHolds, 0);
      await tester.tapAt(const Offset(20, 20));
      await tester.pumpAndSettle();
      expect(
        find.byKey(const ValueKey('reaction-details-sheet')),
        findsNothing,
      );
      debugDefaultTargetPlatformOverride = null;
    },
  );

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
          _reaction(count: 1, userPubkeys: const [_alice]),
        ],
      );

      await tester.longPress(
        find.byKey(const ValueKey('reaction-pill-$_fire')),
      );
      await tester.pumpAndSettle();

      expect(find.byTooltip(':fire:'), findsOneWidget);
      expect(find.byType(TabBar), findsOneWidget);
      expect(find.text('All 1'), findsOneWidget);
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
                  channelId: 'channel',
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
                  channelId: 'channel',
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
