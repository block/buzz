import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/search/recent_searches_provider.dart';
import 'package:buzz/features/search/search_page.dart';
import 'package:buzz/features/search/search_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets(
    'message results build only near the viewport and remain scrollable',
    (tester) async {
      tester.view.physicalSize = const Size(390, 600);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final state = SearchState(
        query: 'result',
        messageResults: [
          for (var index = 0; index < 20; index++)
            SearchHit(
              eventId: 'message-$index',
              content: 'Result **$index** with a preview',
              kind: 9,
              pubkey: 'author-pubkey',
              createdAt: 1,
              score: 1,
            ),
        ],
      );
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            searchProvider.overrideWith(() => _FakeSearchNotifier(state)),
            recentSearchesProvider.overrideWith(
              () => _FakeRecentSearchesNotifier(const []),
            ),
            profileProvider.overrideWith(() => _FakeProfileNotifier()),
            channelsProvider.overrideWith(() => _FakeChannelsNotifier()),
            userCacheProvider.overrideWith(
              () => _FakeUserCacheNotifier(
                const UserProfile(pubkey: 'author-pubkey'),
              ),
            ),
            knownAgentPubkeysProvider.overrideWith((ref) => const {}),
            agentDirectoryDisplayNamesProvider.overrideWith((ref) => const {}),
          ],
          child: const SearchPage(),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byType(MessageContent).evaluate().length, lessThan(10));
      final last = find.byKey(const ValueKey('search-message-row-message-19'));
      expect(last, findsNothing);
      await tester.scrollUntilVisible(
        last,
        400,
        scrollable: find.descendant(
          of: find.byKey(const Key('search-results-list')),
          matching: find.byType(Scrollable),
        ),
      );
      await tester.pumpAndSettle();
      expect(last.hitTestable(), findsOneWidget);
      expect(find.byType(MessageContent).evaluate().length, lessThan(10));
      expect(tester.takeException(), isNull);
    },
  );

  testWidgets(
    'long message previews bound text without parsing partial markdown',
    (tester) async {
      tester.view.physicalSize = const Size(390, 600);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final state = SearchState(
        query: 'result',
        messageResults: [
          SearchHit(
            eventId: 'long',
            content:
                '${'x' * 511}😀${'**[long](https://example.com)**' * 4000}',
            kind: 9,
            pubkey: 'author-pubkey',
            createdAt: 1,
            score: 1,
          ),
        ],
      );
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            searchProvider.overrideWith(() => _FakeSearchNotifier(state)),
            recentSearchesProvider.overrideWith(
              () => _FakeRecentSearchesNotifier(const []),
            ),
            profileProvider.overrideWith(() => _FakeProfileNotifier()),
            channelsProvider.overrideWith(() => _FakeChannelsNotifier()),
            userCacheProvider.overrideWith(
              () => _FakeUserCacheNotifier(
                const UserProfile(pubkey: 'author-pubkey'),
              ),
            ),
            knownAgentPubkeysProvider.overrideWith((ref) => const {}),
            agentDirectoryDisplayNamesProvider.overrideWith((ref) => const {}),
          ],
          child: const SearchPage(),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byType(MessageContent), findsNothing);
      final preview = tester.widget<Text>(
        find.byKey(const ValueKey('search-message-body-long')),
      );
      expect(preview.data, '${'x' * 511}…');
      expect(preview.maxLines, 2);
      expect(preview.overflow, TextOverflow.ellipsis);
      expect(tester.takeException(), isNull);
    },
  );

  for (final category in ['Channels', 'People']) {
    testWidgets('$category results are lazy and reachable after filtering', (
      tester,
    ) async {
      tester.view.physicalSize = const Size(390, 600);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final state = SearchState(
        query: 'result',
        channelResults: [
          for (var index = 0; index < 30; index++)
            Channel(
              id: 'channel-$index',
              name: 'Result $index',
              channelType: 'stream',
              visibility: 'open',
              description: '',
              createdBy: 'test',
              createdAt: DateTime(2025),
              memberCount: 1,
            ),
        ],
        userResults: [
          for (var index = 0; index < 30; index++)
            DirectoryUser(
              pubkey: 'person-$index',
              displayName: 'Person $index',
            ),
        ],
      );
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            searchProvider.overrideWith(() => _FakeSearchNotifier(state)),
            recentSearchesProvider.overrideWith(
              () => _FakeRecentSearchesNotifier(const []),
            ),
            profileProvider.overrideWith(() => _FakeProfileNotifier()),
            channelsProvider.overrideWith(() => _FakeChannelsNotifier()),
          ],
          child: const SearchPage(),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text(category).first);
      await tester.pumpAndSettle();
      final key = category == 'Channels'
          ? 'search-channel-row-channel-29'
          : 'search-person-row-person-29';
      final last = find.byKey(ValueKey(key));
      expect(last, findsNothing);
      await tester.scrollUntilVisible(
        last,
        400,
        scrollable: find.descendant(
          of: find.byKey(const Key('search-results-list')),
          matching: find.byType(Scrollable),
        ),
      );
      await tester.pumpAndSettle();
      expect(last.hitTestable(), findsOneWidget);
      expect(tester.takeException(), isNull);
    });
  }
}

class _FakeSearchNotifier extends SearchNotifier {
  _FakeSearchNotifier(this.initialState);

  final SearchState initialState;

  @override
  SearchState build() => initialState;

  @override
  void search(String query) {
    state = SearchState(query: query.trim());
  }

  @override
  void clear() {
    state = const SearchState.initial();
  }
}

class _FakeRecentSearchesNotifier extends RecentSearchesNotifier {
  _FakeRecentSearchesNotifier(this.initialSearches);

  final List<String> initialSearches;
  List<String> get searches => state;

  @override
  List<String> build() => initialSearches;

  @override
  void record(String query) {
    final trimmed = query.trim();
    if (trimmed.isEmpty) return;
    state = [
      trimmed,
      ...state.where((item) => item.toLowerCase() != trimmed.toLowerCase()),
    ];
  }

  @override
  void clear() {
    state = const [];
  }
}

class _FakeProfileNotifier extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async =>
      const UserProfile(pubkey: 'test', displayName: 'Test');
}

class _FakeChannelsNotifier extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => const [];
}

class _FakeUserCacheNotifier extends UserCacheNotifier {
  _FakeUserCacheNotifier(this.profile);

  final UserProfile profile;

  @override
  Map<String, UserProfile> build() => {profile.pubkey: profile};
}
