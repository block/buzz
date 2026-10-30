import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/slash_commands/groups/presence_commands.dart';
import 'package:buzz/features/profile/user_status.dart';
import 'package:buzz/features/profile/user_status_cache_provider.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/shared/custom_emoji/custom_emoji.dart';
import 'package:buzz/shared/emoji/emoji_data.dart';
import 'package:buzz/shared/emoji/emoji_data_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/buzz_icons.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../../helpers/fake_relay.dart';

const _channelId = '11111111-2222-4333-8444-555555555555';

final _channel = Channel(
  id: _channelId,
  name: 'general',
  channelType: 'stream',
  visibility: 'open',
  description: 'General discussion',
  createdBy: 'abc123',
  createdAt: DateTime(2025),
  memberCount: 5,
  isMember: true,
);

class _Resumed extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

class _EmptyStatusCache extends UserStatusCacheNotifier {
  @override
  Map<String, UserStatus?> build() => {};
}

const _customEmoji = [CustomEmoji(shortcode: 'party', url: 'https://x/p.png')];

final _dataset = EmojiDataset(
  categories: const [],
  all: const [
    EmojiEntry(
      id: 'rocket',
      name: 'Rocket',
      keywords: [],
      native: '🚀',
      categoryId: 'places',
    ),
    EmojiEntry(
      id: '+1',
      name: 'Thumbs Up',
      keywords: [],
      native: '👍',
      categoryId: 'people',
    ),
    EmojiEntry(
      id: '+1',
      name: 'Thumbs Up',
      keywords: [],
      native: '👍🏽',
      categoryId: 'people',
      skinIndex: 3,
    ),
  ],
  nativeToShortcode: const {},
);

void main() {
  testWidgets('/away, /active and /status run from the channel composer', (
    tester,
  ) async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final relay = FakeRelay();
    relay.seed([
      relay.sign(
        kind: kindEmojiSet,
        tags: [
          ['d', customEmojiSetDTag],
          ['emoji', 'party', 'https://x/p.png'],
        ],
      ),
    ]);
    await tester.pumpWidget(
      ProviderScope(
        key: UniqueKey(),
        overrides: [
          ...relay.overrides,
          savedPrefsProvider.overrideWithValue(prefs),
          appLifecycleProvider.overrideWith(_Resumed.new),
          userStatusCacheProvider.overrideWith(_EmptyStatusCache.new),
          emojiDatasetProvider.overrideWith((ref) async => _dataset),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: ChannelDetailPage(channel: _channel),
        ),
      ),
    );
    await tester.pumpAndSettle();
    final container = ProviderScope.containerOf(
      tester.element(find.byType(ChannelDetailPage)),
    );
    final presenceKey = 'buzz_presence_preference_${relay.pubkey}';

    String draft() =>
        tester.widget<TextField>(find.byType(TextField)).controller!.text;
    List<NostrEvent> statuses() =>
        relay.published.where((e) => e.kind == EventKind.userStatus).toList();
    Future<void> run(String text) async {
      if (find.byType(TextField).evaluate().isEmpty) {
        await tester.tap(find.text('Message #general'));
        await tester.pumpAndSettle();
      }
      await tester.enterText(find.byType(TextField), text);
      await tester.pump();
      await tester.tap(find.byIcon(BuzzIcons.arrowUp));
      await tester.pumpAndSettle();
    }

    Future<void> expireToast() async {
      await tester.pump(const Duration(seconds: 6));
      await tester.pumpAndSettle();
    }

    await run('/away');
    expect(find.text("You're away"), findsOneWidget);
    expect(container.read(presenceProvider).value, 'away');
    expect(prefs.getString(presenceKey), 'away');
    expect(
      relay.published
          .where((e) => e.kind == EventKind.presenceUpdate)
          .last
          .content,
      'away',
    );
    expect(draft(), isEmpty);
    expect(
      relay.published.where((e) => e.kind == EventKind.streamMessage),
      isEmpty,
    );
    await expireToast();

    await run('/active');
    expect(find.text("You're active"), findsOneWidget);
    expect(container.read(presenceProvider).value, 'online');
    expect(prefs.getString(presenceKey), 'auto');
    await expireToast();

    await run('/status 🚀 shipping the thing');
    expect(find.text('Status set'), findsOneWidget);
    expect(statuses().single.content, 'shipping the thing');
    expect(statuses().single.getTagValue('emoji'), '🚀');
    expect(statuses().single.getTagValue('expiration'), isNull);
    expect(draft(), isEmpty);
    await expireToast();

    await run('/status :party: launch');
    expect(statuses().last.getTagValue('emoji'), ':party:');
    expect(statuses().last.content, 'launch');
    await expireToast();

    await run('/status :Rocket: liftoff');
    expect(statuses().last.getTagValue('emoji'), '🚀');
    await expireToast();

    await run('/status heads down');
    expect(statuses().last.content, 'heads down');
    expect(statuses().last.getTagValue('emoji'), isNull);
    await expireToast();

    await run('/status clear');
    expect(find.text('Status cleared'), findsOneWidget);
    expect(statuses().last.content, isEmpty);
    expect(statuses().last.getTagValue('emoji'), isNull);
    await expireToast();

    final before = statuses().length;
    await run('/status');
    expect(find.text('Set a status'), findsOneWidget);
    expect(statuses().length, before);
    expect(draft(), isEmpty);
    await tester.tapAt(const Offset(195, 20));
    await tester.pumpAndSettle();
    expect(find.text('Set a status'), findsNothing);

    relay.rejectNext(
      'blocked: not allowed',
      match: (e) => e.kind == EventKind.userStatus,
    );
    await run('/status 🔥 on fire');
    expect(draft(), '/status 🔥 on fire');
    expect(find.byType(SnackBar), findsOneWidget);
    expect(find.text('Status set'), findsNothing);
    await expireToast();

    await run('/away now');
    expect(draft(), '/away now');
    expect(find.textContaining('Usage: /away'), findsOneWidget);
    expect(container.read(presenceProvider).value, 'online');
  });

  group('splitStatusArgs', () {
    const cases = <(String, String, String)>[
      ('🚀 shipping', '🚀', 'shipping'),
      ('🚀', '🚀', ''),
      ('  🚀   shipping   it ', '🚀', 'shipping   it'),
      ('👍🏽 nice', '👍🏽', 'nice'),
      ('🇺🇸 abroad', '🇺🇸', 'abroad'),
      ('👨‍👩‍👧 family time', '👨‍👩‍👧', 'family time'),
      ('1️⃣ first', '1️⃣', 'first'),
      ('❤️ love', '❤️', 'love'),
      (':party: launch', ':party:', 'launch'),
      (':PARTY: launch', ':party:', 'launch'),
      (':rocket: launch', '🚀', 'launch'),
      (':+1: nice', '👍', 'nice'),
      (':nope: launch', '', ':nope: launch'),
      (':rocket:launch', '', ':rocket:launch'),
      ('::', '', '::'),
      ('🚀🚀 double', '', '🚀🚀 double'),
      ('🚀shipping', '', '🚀shipping'),
      ('1 thing', '', '1 thing'),
      ('# heading', '', '# heading'),
      ('a status', '', 'a status'),
      ('out :rocket: now', '', 'out :rocket: now'),
      ('in a meeting 🚀', '', 'in a meeting 🚀'),
    ];

    for (final (args, emoji, text) in cases) {
      test('"$args"', () {
        final parts = splitStatusArgs(
          args,
          customEmoji: _customEmoji,
          dataset: _dataset,
        );
        expect(parts, (emoji: emoji, text: text));
      });
    }

    test('a shortcode needs the dataset only when it leads the arguments', () {
      expect(statusArgsNeedEmojiDataset(':rocket: go'), isTrue);
      expect(statusArgsNeedEmojiDataset('🚀 go'), isFalse);
      expect(statusArgsNeedEmojiDataset('go :rocket:'), isFalse);
    });
  });
}
