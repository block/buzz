import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/features/search/search_page.dart';
import 'package:buzz/features/search/search_provider.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

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
    expect(tester.getSize(message).height, greaterThan(32));
    expect(tester.takeException(), isNull);
  });

  testWidgets('keeps search results scrollable above the keyboard', (
    tester,
  ) async {
    const keyboardInset = 300.0;
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

    final results = tester.widget<ListView>(
      find.byKey(const Key('search-results-list')),
    );
    final padding = results.padding! as EdgeInsets;

    expect(padding.bottom, Grid.xl + keyboardInset);
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
}

class _FakeSearchNotifier extends SearchNotifier {
  _FakeSearchNotifier(this.initialState);

  final SearchState initialState;

  @override
  SearchState build() => initialState;
}

class _FakeProfileNotifier extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async =>
      const UserProfile(pubkey: 'test', displayName: 'Test');
}
