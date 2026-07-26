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
