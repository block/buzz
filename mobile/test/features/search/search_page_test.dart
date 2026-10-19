import 'package:buzz/shared/widgets/buzz_agent_icon.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/channels/small_avatar.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/features/search/recent_searches_provider.dart';
import 'package:buzz/features/search/search_page.dart';
import 'package:buzz/features/search/search_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/widgets/concentric_sheet_surface.dart';
import 'package:buzz/shared/widgets/ios_glass_navigation_button.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';

import '../../helpers/widget_helpers.dart';

void main() {
  for (final platform in [TargetPlatform.android, TargetPlatform.iOS]) {
    for (final scale in [1.0, 2.0]) {
      testWidgets('search controls stay fixed on $platform at scale $scale', (
        tester,
      ) async {
        debugDefaultTargetPlatformOverride = platform;
        addTearDown(() => debugDefaultTargetPlatformOverride = null);
        tester.view.physicalSize = const Size(390, 844);
        tester.view.devicePixelRatio = 1;
        addTearDown(tester.view.reset);
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
            child: MediaQuery(
              data: MediaQueryData(
                textScaler: TextScaler.linear(scale),
                padding: const EdgeInsets.only(top: 59),
              ),
              child: const SearchPage(),
            ),
          ),
        );
        await tester.pumpAndSettle();
        final field = find.byKey(const Key('search-field-container'));
        final filters = find.byKey(const Key('search-header-filters'));
        final close = find.byKey(const Key('search-close'));
        final fieldRect = tester.getRect(field);
        final filtersRect = tester.getRect(filters);
        expect(
          find.descendant(of: field, matching: find.text('Search')),
          findsOneWidget,
        );
        expect(find.text('Cancel'), findsNothing);
        expect(fieldRect.top, greaterThanOrEqualTo(59));
        expect(tester.getRect(close).right, lessThan(fieldRect.left));
        expect(tester.getSize(close), const Size(48, 48));
        for (final label in ['All', 'Messages', 'Channels', 'People']) {
          expect(
            platform == TargetPlatform.iOS
                ? find.byTooltip(label)
                : find.text(label).hitTestable(),
            findsOneWidget,
          );
        }
        final input = tester.element(find.byType(TextField));
        await tester.tap(find.byType(TextField));
        await tester.pumpAndSettle();
        expect(tester.getRect(field), fieldRect);
        expect(tester.getRect(filters), filtersRect);
        expect(tester.element(find.byType(TextField)), same(input));
        expect(
          tester.widget<TextField>(find.byType(TextField)).focusNode!.hasFocus,
          isTrue,
        );
        await tester.enterText(find.byType(TextField), 'design');
        if (platform == TargetPlatform.iOS) {
          await _pressNativeFilter(tester, 'Channels');
        } else {
          await tester.tap(find.text('Channels'));
        }
        await tester.pumpAndSettle();
        expect(tester.getRect(field), fieldRect);
        expect(tester.getRect(filters), filtersRect);
        expect(
          tester
              .widget<ConcentricSheetSurface>(
                find.byType(ConcentricSheetSurface),
              )
              .usesGlass,
          isTrue,
        );
        expect(
          find.byType(IosGlassNavigationButton),
          platform == TargetPlatform.iOS ? findsNWidgets(5) : findsNothing,
        );
        expect(tester.takeException(), isNull);
        await tester.pumpWidget(const SizedBox.shrink());
        debugDefaultTargetPlatformOverride = null;
      });
    }
  }

  testWidgets('iOS native filter press selects Channels and filters results', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final state = SearchState(
      query: 'design',
      channelResults: [
        Channel(
          id: 'design',
          name: 'design',
          channelType: 'stream',
          visibility: 'open',
          description: 'Design discussion',
          memberCount: 4,
          createdBy: 'test',
          createdAt: DateTime(2025),
          isMember: true,
        ),
      ],
      userResults: const [DirectoryUser(pubkey: 'maya', displayName: 'Maya')],
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
    final channel = find.byKey(const ValueKey('search-channel-row-design'));
    final person = find.byKey(const ValueKey('search-person-row-maya'));
    expect(channel, findsOneWidget);
    expect(person, findsOneWidget);
    expect(
      tester.widget<IosGlassNavigationButton>(_nativeFilter('All')).isSelected,
      isTrue,
    );
    expect(
      tester
          .widget<IosGlassNavigationButton>(_nativeFilter('Channels'))
          .isSelected,
      isFalse,
    );

    await _pressNativeFilter(tester, 'Channels');
    await tester.pumpAndSettle();

    expect(
      tester
          .widget<IosGlassNavigationButton>(_nativeFilter('Channels'))
          .isSelected,
      isTrue,
    );
    expect(
      tester.widget<IosGlassNavigationButton>(_nativeFilter('All')).isSelected,
      isFalse,
    );
    expect(channel, findsOneWidget);
    expect(person, findsNothing);
    expect(tester.takeException(), isNull);
    await tester.pumpWidget(const SizedBox.shrink());
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('X dismisses search while the keyboard is focused', (
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
        child: Builder(
          builder: (context) => TextButton(
            onPressed: () => Navigator.of(
              context,
            ).push(MaterialPageRoute<void>(builder: (_) => const SearchPage())),
            child: const Text('Open search'),
          ),
        ),
      ),
    );
    await tester.tap(find.text('Open search'));
    await tester.pumpAndSettle();
    await tester.tap(find.byType(TextField));
    await tester.pumpAndSettle();
    await tester.enterText(find.byType(TextField), 'draft');
    await tester.tap(find.byTooltip('Close search'));
    await tester.pumpAndSettle();
    expect(find.byType(SearchPage), findsNothing);
    expect(find.text('Open search'), findsOneWidget);
  });

  testWidgets('reselecting Search restores input focus without moving tabs', (
    tester,
  ) async {
    final reselection = ValueNotifier(0);
    addTearDown(reselection.dispose);
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
        child: SearchPage(tabReselection: reselection),
      ),
    );
    await tester.pumpAndSettle();
    final filters = tester.getRect(
      find.byKey(const Key('search-header-filters')),
    );
    reselection.value++;
    await tester.pumpAndSettle();
    final focus = tester.widget<TextField>(find.byType(TextField)).focusNode!;
    expect(focus.hasFocus, isTrue);
    reselection.value++;
    focus.unfocus();
    await tester.pumpAndSettle();
    expect(focus.hasFocus, isTrue);
    expect(
      tester.getRect(find.byKey(const Key('search-header-filters'))),
      filters,
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
    await tester.enterText(find.byKey(const Key('search-field')), '');
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('recent-searches-list')), findsNothing);

    await tester.tap(searchField);
    await tester.pumpAndSettle();
    await tester.enterText(searchField, 'design systems');
    await tester.testTextInput.receiveAction(TextInputAction.search);
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('search-close')), findsOneWidget);
    // Submission dismisses editing immediately. Reopen before clearing.
    await tester.tap(searchField);
    await tester.pumpAndSettle();
    await tester.enterText(find.byKey(const Key('search-field')), '');
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

    await tester.enterText(find.byKey(const Key('search-field')), '');
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('recent-search-0')), findsOneWidget);
    expect(find.text('launch plan'), findsOneWidget);

    await tester.tap(find.byKey(const Key('clear-recent-searches')));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('recent-searches-list')), findsNothing);
    expect(
      find.descendant(
        of: find.byKey(const Key('search-empty-state')),
        matching: find.text('Search messages, channels, and people'),
      ),
      findsOneWidget,
    );
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

    final resultsPadding = tester.widget<SliverPadding>(
      find.descendant(
        of: find.byKey(const Key('search-results-list')),
        matching: find.byType(SliverPadding),
      ),
    );
    final padding = resultsPadding.padding as EdgeInsets;

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

  testWidgets('same-name people results get distinct labels', (tester) async {
    final first = 'a' * 64, second = 'b' * 64;
    final state = SearchState(
      query: 'scout',
      channelResults: const [],
      userResults: [
        DirectoryUser(pubkey: first, displayName: 'Scout'),
        DirectoryUser(pubkey: second, displayName: 'Scout'),
      ],
      messageResults: const [],
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
              UserProfile(pubkey: first, displayName: 'Scout'),
            ),
          ),
        ],
        child: const SearchPage(),
      ),
    );
    await tester.pumpAndSettle();

    String title(String pubkey) => tester
        .widget<Text>(find.byKey(ValueKey('search-person-title-$pubkey')))
        .data!;
    expect(title(first), isNot(title(second)));
    expect(title(first), isNot('Scout'));
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
    // Mention tags carry exact 64-hex keys; readers ignore anything else.
    final agentPubkey = 'a9' * 32;
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
            (ref) => {agentPubkey: 'Helper Bot'},
          ),
        ],
        child: const SearchPage(),
      ),
    );
    await tester.pumpAndSettle();

    final content = tester.widget<MessageContent>(
      find.byKey(const ValueKey('search-message-body-message-1')),
    );
    expect(content.mentionNames, {agentPubkey: 'Helper Bot'});
    expect(content.agentMentionPubkeys, contains(agentPubkey));
    expect(find.byType(BuzzAgentIcon), findsOneWidget);
  });

  testWidgets('does not label an unjoined channel as having zero members', (
    tester,
  ) async {
    final state = SearchState(
      query: 'community',
      channelResults: [
        Channel(
          id: 'community-help',
          name: 'community-help',
          channelType: 'stream',
          visibility: 'open',
          description: 'Help from the community',
          createdBy: 'test',
          createdAt: DateTime(2025),
          memberCount: 0,
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
        child: const SearchPage(),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Open'), findsOneWidget);
    expect(find.text('0 members'), findsNothing);
  });
}

Finder _nativeFilter(String label) => find.byWidgetPredicate(
  (widget) => widget is IosGlassNavigationButton && widget.label == label,
);

Future<void> _pressNativeFilter(WidgetTester tester, String label) async {
  final view = tester.widget<UiKitView>(
    find.descendant(of: _nativeFilter(label), matching: find.byType(UiKitView)),
  );
  // Simulate the engine creating this view, then deliver the native tap through
  // the same per-view channel used by the production UIKit control.
  const viewId = 42;
  view.onPlatformViewCreated!(viewId);
  await tester.pump();
  const channel = MethodChannel('buzz/navigation_glass/$viewId');
  await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
    channel.name,
    channel.codec.encodeMethodCall(const MethodCall('pressed')),
    (_) {},
  );
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
