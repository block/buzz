import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/small_avatar.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/user_cache_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/features/search/recent_searches_provider.dart';
import 'package:buzz/features/search/search_page.dart';
import 'package:buzz/features/search/search_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  testWidgets('empty state preserves large accessible text scaling', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(320, 640);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          searchProvider.overrideWith(
            () => _FakeSearchNotifier(const SearchState.initial()),
          ),
          recentSearchesProvider.overrideWith(
            () => _FakeRecentSearchesNotifier(const []),
          ),
          profileProvider.overrideWith(() => _FakeProfileNotifier()),
        ],
        child: Builder(
          builder: (context) => MediaQuery(
            data: MediaQuery.of(
              context,
            ).copyWith(textScaler: const TextScaler.linear(2)),
            child: const SearchPage(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    final emptyState = find.byKey(const Key('search-empty-state'));
    final message = find.text('Search messages, channels, and people');
    final searchField = find.byKey(const Key('search-field-container'));
    final searchFieldContext = tester.element(searchField);
    final bodyStyle = Theme.of(searchFieldContext).textTheme.bodyMedium!;
    final scaledLineHeight =
        MediaQuery.textScalerOf(searchFieldContext).scale(bodyStyle.fontSize!) *
        bodyStyle.height!;

    expect(
      find.descendant(of: emptyState, matching: find.byType(FittedBox)),
      findsNothing,
    );
    expect(
      tester.getSize(searchField).height,
      greaterThanOrEqualTo(scaledLineHeight + Grid.xxs * 2),
    );
    final input = tester.widget<TextField>(
      find.byKey(const Key('search-field')),
    );
    expect(input.style?.fontSize, searchInputTextStyle.fontSize);
    expect(input.style?.height, searchInputTextStyle.height);
    expect(tester.getSize(message).height, greaterThan(32));
    expect(tester.takeException(), isNull);
  });

  testWidgets('focus slides Cancel in beside the search field', (tester) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          searchProvider.overrideWith(
            () => _FakeSearchNotifier(const SearchState.initial()),
          ),
          recentSearchesProvider.overrideWith(
            () => _FakeRecentSearchesNotifier(const []),
          ),
          profileProvider.overrideWith(() => _FakeProfileNotifier()),
        ],
        child: const SearchPage(),
      ),
    );
    await tester.pumpAndSettle();

    final searchField = find.byKey(const Key('search-field'));
    final searchFieldContainer = find.byKey(
      const Key('search-field-container'),
    );
    final unfocusedWidth = tester.getSize(searchFieldContainer).width;
    expect(find.byKey(const Key('search-cancel')), findsNothing);
    expect(
      tester.widget<TextField>(searchField).decoration?.hintText,
      'Search messages, channels, people\u2026',
    );
    expect(
      tester.widget<TextField>(searchField).textInputAction,
      TextInputAction.search,
    );

    await tester.tap(searchField);
    await tester.pump();

    final cancel = find.byKey(const Key('search-cancel'));
    expect(cancel, findsOneWidget);
    expect(
      tester.getSize(cancel).height,
      greaterThanOrEqualTo(Grid.xl),
      reason: 'Cancel must keep a 48dp touch target.',
    );
    expect(tester.widget<TextField>(searchField).decoration?.hintText, isNull);
    final enteringSlide = tester.widget<SlideTransition>(
      find.ancestor(of: cancel, matching: find.byType(SlideTransition)).first,
    );
    expect(enteringSlide.position.value.dx, greaterThan(0));

    await tester.pump(const Duration(milliseconds: 160));
    final focusedWidth = tester.getSize(searchFieldContainer).width;
    expect(focusedWidth, lessThan(unfocusedWidth));
    final settledSlide = tester.widget<SlideTransition>(
      find.ancestor(of: cancel, matching: find.byType(SlideTransition)).first,
    );
    expect(settledSlide.position.value, Offset.zero);

    await tester.enterText(searchField, 'design');
    await tester.tap(cancel);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 60));
    final exitingWidth = tester.getSize(searchFieldContainer).width;
    expect(exitingWidth, greaterThan(focusedWidth));
    expect(exitingWidth, lessThan(unfocusedWidth));
    await tester.pumpAndSettle();

    final input = tester.widget<TextField>(searchField);
    expect(input.controller?.text, isEmpty);
    expect(input.focusNode?.hasFocus, isFalse);
    expect(
      input.decoration?.hintText,
      'Search messages, channels, people\u2026',
    );
    expect(find.byKey(const Key('search-cancel')), findsNothing);
    expect(
      tester.getSize(searchFieldContainer).width,
      closeTo(unfocusedWidth, 0.01),
    );
  });

  testWidgets('only submitted queries are added to recent searches', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          searchProvider.overrideWith(
            () => _FakeSearchNotifier(const SearchState.initial()),
          ),
          recentSearchesProvider.overrideWith(
            () => _FakeRecentSearchesNotifier(const []),
          ),
          profileProvider.overrideWith(() => _FakeProfileNotifier()),
        ],
        child: const SearchPage(),
      ),
    );
    await tester.pumpAndSettle();

    final searchField = find.byKey(const Key('search-field'));
    await tester.tap(searchField);
    await tester.pumpAndSettle();
    await tester.enterText(searchField, 'draft');
    await tester.tap(find.byKey(const Key('search-cancel')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('recent-searches-list')), findsNothing);

    await tester.tap(searchField);
    await tester.pumpAndSettle();
    await tester.enterText(searchField, 'design systems');
    await tester.testTextInput.receiveAction(TextInputAction.search);
    await tester.pump();
    await tester.tap(find.byKey(const Key('search-cancel')));
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('recent-searches-list')), findsOneWidget);
    expect(find.text('design systems'), findsOneWidget);
  });

  testWidgets('recent searches can be rerun and cleared', (tester) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          searchProvider.overrideWith(
            () => _FakeSearchNotifier(const SearchState.initial()),
          ),
          recentSearchesProvider.overrideWith(
            () => _FakeRecentSearchesNotifier(const [
              'design systems',
              'launch plan',
            ]),
          ),
          profileProvider.overrideWith(() => _FakeProfileNotifier()),
        ],
        child: const SearchPage(),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('recent-searches-list')), findsOneWidget);
    expect(find.text('Recent searches'), findsOneWidget);
    expect(find.text('design systems'), findsOneWidget);
    expect(find.text('launch plan'), findsOneWidget);
    expect(
      tester.getSize(find.byKey(const ValueKey('recent-search-0'))).height,
      greaterThanOrEqualTo(Grid.xl),
    );
    expect(
      tester.getSize(find.byKey(const ValueKey('recent-search-1'))).height,
      greaterThanOrEqualTo(Grid.xl),
    );

    await tester.tap(find.byKey(const ValueKey('recent-search-1')));
    await tester.pumpAndSettle();

    final searchField = find.byKey(const Key('search-field'));
    final input = tester.widget<TextField>(searchField);
    expect(input.controller?.text, 'launch plan');
    expect(input.focusNode?.hasFocus, isTrue);
    expect(find.text("No results for 'launch plan'"), findsOneWidget);

    await tester.tap(find.byKey(const Key('search-cancel')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('recent-search-0')), findsOneWidget);
    expect(find.text('launch plan'), findsOneWidget);

    await tester.tap(find.byKey(const Key('clear-recent-searches')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('recent-searches-list')), findsNothing);
    expect(find.text('Search messages, channels, and people'), findsOneWidget);
  });

  testWidgets('keeps recent searches scrollable above the keyboard', (
    tester,
  ) async {
    const keyboardInset = 300.0;
    const footerClearance = 102.0;

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          searchProvider.overrideWith(
            () => _FakeSearchNotifier(const SearchState.initial()),
          ),
          recentSearchesProvider.overrideWith(
            () => _FakeRecentSearchesNotifier(const [
              'design systems',
              'launch plan',
            ]),
          ),
          profileProvider.overrideWith(() => _FakeProfileNotifier()),
        ],
        child: Builder(
          builder: (context) => MediaQuery(
            data: MediaQuery.of(context).copyWith(
              padding: const EdgeInsets.only(bottom: footerClearance),
              viewInsets: const EdgeInsets.only(bottom: keyboardInset),
            ),
            child: const SearchPage(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('search-field')));
    await tester.pumpAndSettle();

    final recentSearches = tester.widget<ListView>(
      find.byKey(const Key('recent-searches-list')),
    );
    final padding = recentSearches.padding! as EdgeInsets;

    expect(padding.bottom, Grid.xl + footerClearance + keyboardInset);
  });

  testWidgets('keeps search results scrollable above the keyboard', (
    tester,
  ) async {
    const keyboardInset = 300.0;
    const footerClearance = 102.0;
    final state = SearchState(
      query: 'general',
      channelResults: [
        Channel(
          id: 'general',
          name: 'general',
          channelType: 'stream',
          visibility: 'open',
          description: 'General discussion',
          createdBy: 'test',
          createdAt: DateTime(2025),
          memberCount: 1,
          isMember: true,
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
        ],
        child: Builder(
          builder: (context) => MediaQuery(
            data: MediaQuery.of(context).copyWith(
              padding: const EdgeInsets.only(bottom: footerClearance),
              viewInsets: const EdgeInsets.only(bottom: keyboardInset),
            ),
            child: const SearchPage(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    final results = tester.widget<ListView>(
      find.byKey(const Key('search-results-list')),
    );
    final padding = results.padding! as EdgeInsets;

    expect(padding.bottom, Grid.xl + footerClearance + keyboardInset);
  });

  testWidgets('keeps no-results feedback above the keyboard', (tester) async {
    const keyboardInset = 300.0;
    tester.view.physicalSize = const Size(320, 640);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);

    const query = 'missing';
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          searchProvider.overrideWith(
            () => _FakeSearchNotifier(const SearchState(query: query)),
          ),
          recentSearchesProvider.overrideWith(
            () => _FakeRecentSearchesNotifier(const []),
          ),
          profileProvider.overrideWith(() => _FakeProfileNotifier()),
        ],
        child: Builder(
          builder: (context) => MediaQuery(
            data: MediaQuery.of(context).copyWith(
              viewInsets: const EdgeInsets.only(bottom: keyboardInset),
            ),
            child: const SearchPage(),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();

    final noResults = find.byKey(const Key('search-no-results-state'));
    final message = find.text("No results for '$query'");
    final keyboardTop =
        tester.view.physicalSize.height / tester.view.devicePixelRatio -
        keyboardInset;

    expect(noResults, findsOneWidget);
    expect(tester.getBottomLeft(message).dy, lessThan(keyboardTop));
    expect(tester.takeException(), isNull);
  });

  testWidgets('uses compact content styles and keeps message time by author', (
    tester,
  ) async {
    late _FakeRecentSearchesNotifier recentSearches;
    final createdAt = DateTime.now().millisecondsSinceEpoch ~/ 1000 - 120;
    final state = SearchState(
      query: 'design',
      channelResults: [
        Channel(
          id: 'design',
          name: 'design',
          channelType: 'stream',
          visibility: 'open',
          description: 'Design discussion',
          createdBy: 'test',
          createdAt: DateTime(2025),
          memberCount: 4,
          isMember: true,
        ),
      ],
      userResults: const [
        DirectoryUser(
          pubkey: 'maya',
          displayName: 'Maya',
          nip05Handle: 'maya@example.com',
        ),
      ],
      messageResults: [
        SearchHit(
          eventId: 'message-1',
          content: 'The latest design is ready',
          kind: 9,
          pubkey: 'alice',
          channelName: 'design',
          createdAt: createdAt,
          score: 1,
        ),
      ],
    );

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          searchProvider.overrideWith(() => _FakeSearchNotifier(state)),
          recentSearchesProvider.overrideWith(
            () => recentSearches = _FakeRecentSearchesNotifier(const []),
          ),
          profileProvider.overrideWith(() => _FakeProfileNotifier()),
          channelsProvider.overrideWith(() => _FakeChannelsNotifier()),
          userCacheProvider.overrideWith(
            () => _FakeUserCacheNotifier(
              const UserProfile(
                pubkey: 'alice',
                displayName: 'Alice',
                nip05Handle: 'alice@example.com',
              ),
            ),
          ),
        ],
        child: const SearchPage(),
      ),
    );
    await tester.pumpAndSettle();

    final channelTitle = tester.widget<Text>(
      find.byKey(const ValueKey('search-channel-title-design')),
    );
    final personTitle = tester.widget<Text>(
      find.byKey(const ValueKey('search-person-title-maya')),
    );
    for (final title in [channelTitle, personTitle]) {
      expect(title.style?.fontSize, contentListTitleTextStyle.fontSize);
      expect(title.style?.fontWeight, contentListTitleTextStyle.fontWeight);
      expect(title.style?.height, contentListTitleTextStyle.height);
    }
    for (final label in ['channels', 'people', 'messages']) {
      final sectionLabel = tester.widget<Text>(
        find.byKey(ValueKey('search-section-$label')),
      );
      expect(
        sectionLabel.data,
        '${label[0].toUpperCase()}${label.substring(1)}',
      );
      expect(sectionLabel.style?.fontSize, activityContextTextStyle.fontSize);
      expect(
        sectionLabel.style?.fontWeight,
        activityContextTextStyle.fontWeight,
      );
      expect(sectionLabel.style?.letterSpacing, 0);
    }
    for (final rowKey in [
      'search-channel-row-design',
      'search-person-row-maya',
      'search-message-row-message-1',
    ]) {
      final row = tester.widget<ListTile>(find.byKey(ValueKey(rowKey)));
      expect(
        row.contentPadding,
        const EdgeInsets.symmetric(horizontal: Grid.gutter),
      );
    }
    for (final alignment in [
      ('channels', 'search-channel-leading-design'),
      ('people', 'search-person-leading-maya'),
      ('messages', 'search-message-avatar-message-1'),
    ]) {
      expect(
        tester
            .getTopLeft(find.byKey(ValueKey('search-section-${alignment.$1}')))
            .dx,
        tester.getTopLeft(find.byKey(ValueKey(alignment.$2))).dx,
      );
    }

    final authorFinder = find.byKey(
      const ValueKey('search-message-author-message-1'),
    );
    final usernameFinder = find.byKey(
      const ValueKey('search-message-username-message-1'),
    );
    final timestampFinder = find.byKey(
      const ValueKey('search-message-timestamp-message-1'),
    );
    final author = tester.widget<Text>(authorFinder);
    final username = tester.widget<Text>(usernameFinder);
    final timestamp = tester.widget<Text>(timestampFinder);
    expect(author.style?.fontSize, messageUsernameTextStyle.fontSize);
    expect(author.style?.fontWeight, messageUsernameTextStyle.fontWeight);
    expect(author.style?.height, messageUsernameTextStyle.height);
    expect(username.data, 'alice@example.com');
    expect(username.style?.fontSize, messageMetadataTextStyle.fontSize);
    expect(username.style?.fontWeight, FontWeight.w400);
    expect(username.style?.height, messageMetadataTextStyle.height);
    expect(timestamp.style?.fontSize, messageTimestampTextStyle.fontSize);
    expect(timestamp.style?.height, messageTimestampTextStyle.height);
    expect(
      (tester.getCenter(authorFinder).dy - tester.getCenter(timestampFinder).dy)
          .abs(),
      lessThan(1),
    );
    expect(
      (tester
                  .getTopLeft(
                    find.byKey(
                      const ValueKey('search-message-avatar-message-1'),
                    ),
                  )
                  .dy -
              tester.getTopLeft(authorFinder).dy)
          .abs(),
      lessThan(6),
    );

    final body = tester.widget<MessageContent>(
      find.byKey(const ValueKey('search-message-body-message-1')),
    );
    expect(body.baseStyle?.fontSize, activityPreviewTextStyle.fontSize);
    expect(body.baseStyle?.height, activityPreviewTextStyle.height);
    expect(
      tester.widget<SmallAvatar>(find.byType(SmallAvatar)).size,
      compactMessageAvatarSize,
    );
    final contextLabel = tester.widget<Text>(find.text('Message in'));
    final channelLabel = tester.widget<Text>(
      find.byKey(const ValueKey('search-message-channel-message-1')),
    );
    expect(contextLabel.style?.fontSize, activityContextTextStyle.fontSize);
    expect(contextLabel.style?.height, activityContextTextStyle.height);
    expect(channelLabel.data, '#design');
    expect(channelLabel.style?.fontSize, activityContextTextStyle.fontSize);
    final channelChip = tester.widget<Container>(
      find.byWidgetPredicate(
        (widget) =>
            widget is Container &&
            widget.child is Text &&
            (widget.child as Text).key ==
                const ValueKey('search-message-channel-message-1'),
      ),
    );
    expect(
      (channelChip.decoration! as BoxDecoration).borderRadius,
      BorderRadius.circular(Radii.xs),
    );
    expect(
      tester
          .getTopLeft(
            find.byKey(const ValueKey('search-message-context-message-1')),
          )
          .dy,
      greaterThan(tester.getTopLeft(authorFinder).dy),
    );
    expect(
      tester
          .getTopLeft(
            find.byKey(const ValueKey('search-message-body-message-1')),
          )
          .dy,
      greaterThan(
        tester
            .getBottomLeft(
              find.byKey(const ValueKey('search-message-context-message-1')),
            )
            .dy,
      ),
    );

    await tester.tap(
      find.byKey(const ValueKey('search-message-row-message-1')),
    );
    await tester.pump();
    expect(recentSearches.searches, const ['design']);
  });

  testWidgets('renders channel-role bots in message previews', (tester) async {
    final channel = Channel(
      id: 'channel-1',
      name: 'general',
      channelType: 'stream',
      visibility: 'open',
      description: '',
      createdBy: 'test',
      createdAt: DateTime(2025),
      memberCount: 2,
      isMember: true,
    );
    const agentPubkey = 'agent-pubkey';
    const cachedProfile = UserProfile(pubkey: 'author-pubkey');
    final state = SearchState(
      query: 'helper',
      messageResults: [
        SearchHit(
          eventId: 'message-1',
          content: 'Ask @Helper Bot to investigate',
          kind: 9,
          pubkey: 'author-pubkey',
          channelId: channel.id,
          channelName: channel.name,
          createdAt: 1,
          score: 1,
          tags: [
            ['p', agentPubkey],
          ],
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
          channelsProvider.overrideWith(() => _FakeChannelsNotifier([channel])),
          userCacheProvider.overrideWith(
            () => _FakeUserCacheNotifier(cachedProfile),
          ),
          knownAgentPubkeysProvider.overrideWith((ref) => const {}),
          channelBotPubkeysProvider(
            channel.id,
          ).overrideWith((ref) async => {agentPubkey}),
          agentDirectoryDisplayNamesProvider.overrideWith(
            (ref) => const {agentPubkey: 'Helper Bot'},
          ),
        ],
        child: const SearchPage(),
      ),
    );
    await tester.pumpAndSettle();

    final content = tester.widget<MessageContent>(
      find.byKey(const ValueKey('search-message-body-message-1')),
    );
    expect(content.mentionNames, const {agentPubkey: 'Helper Bot'});
    expect(content.agentMentionPubkeys, contains(agentPubkey));
    expect(find.byIcon(LucideIcons.bot), findsOneWidget);
  });
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
  _FakeChannelsNotifier([this.channels = const []]);

  final List<Channel> channels;

  @override
  Future<List<Channel>> build() async => channels;
}

class _FakeUserCacheNotifier extends UserCacheNotifier {
  _FakeUserCacheNotifier(this.profile);

  final UserProfile profile;

  @override
  Map<String, UserProfile> build() => {profile.pubkey: profile};
}
