import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/search/recent_searches_provider.dart';
import 'package:buzz/features/search/search_page.dart';
import 'package:buzz/features/search/search_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/widgets/buzz_loading_indicator.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/widget_helpers.dart';

// Channel search must find open channels the user has not joined, rank names
// like desktop, and stop its spinner once every lookup is done, whichever
// lookup finishes last.
void main() {
  testWidgets('search finds unjoined open channels and settles its spinner', (
    tester,
  ) async {
    final session = _HeldSession();
    final actions = _HeldUserSearch();
    final channels = _DirectoryChannels();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          relayConfigProvider.overrideWith(_Community.new),
          relaySessionProvider.overrideWith(() => session),
          channelActionsProvider.overrideWith((ref) => actions),
          channelsProvider.overrideWith(() => channels),
          recentSearchesProvider.overrideWith(_NoRecentSearches.new),
          profileProvider.overrideWith(_Profile.new),
        ],
        child: const SearchPage(),
      ),
    );
    await tester.pumpAndSettle();

    final spinner = find.byType(BuzzLoadingIndicator);
    final unjoined = find.byKey(
      const ValueKey('search-channel-row-4things-id'),
    );

    // The open channel is not in the member list, so it needs the directory.
    await tester.enterText(find.byType(TextField), '4things');
    await tester.pump(const Duration(milliseconds: 300));
    expect(channels.directoryRequests, 1);
    // Later searches read the directory again once it is this old.
    expect(channels.maxAge, searchDirectoryMaxAge);
    expect(spinner, findsOneWidget);
    expect(unjoined, findsNothing);

    final joined = find.byKey(
      const ValueKey('search-channel-row-team-4things-id'),
    );
    expect(joined, findsOneWidget);

    channels.finishDirectory();
    await tester.pump();
    expect(unjoined, findsOneWidget);
    // Match rank comes before membership: the exact name leads.
    expect(
      tester.getTopLeft(unjoined).dy,
      lessThan(tester.getTopLeft(joined).dy),
    );
    expect(
      find.descendant(of: unjoined, matching: find.text('Open')),
      findsOneWidget,
    );
    // Messages and people are still searching.
    expect(spinner, findsOneWidget);

    session.searches.removeAt(0).complete(const []);
    await tester.pump();
    expect(spinner, findsOneWidget);
    // People finish last with nothing. The spinner must still stop.
    actions.searches.removeAt(0).complete(const []);
    await tester.pump();
    expect(spinner, findsNothing);
    expect(unjoined, findsOneWidget);

    // Letters in order still find the member channel, like desktop.
    await tester.enterText(find.byType(TextField), 'genral');
    await tester.pump(const Duration(milliseconds: 300));
    // The directory is already loaded for this community.
    expect(channels.directoryRequests, 1);
    expect(
      find.byKey(const ValueKey('search-channel-row-general-id')),
      findsOneWidget,
    );
    session.searches.removeAt(0).complete(const []);
    actions.searches.removeAt(0).complete(const []);
    await tester.pump();
    expect(spinner, findsNothing);

    // Nothing matches anywhere and people finish last: no endless spinner.
    await tester.enterText(find.byType(TextField), 'zzzz');
    await tester.pump(const Duration(milliseconds: 300));
    session.searches.removeAt(0).complete(const []);
    await tester.pump();
    actions.searches.removeAt(0).complete(const []);
    await tester.pump();
    expect(spinner, findsNothing);
    expect(find.text("No results for 'zzzz'"), findsOneWidget);

    // Clearing and retyping the same text starts new lookups. Late replies to
    // the first ones must not settle the second search's spinner.
    await tester.enterText(find.byType(TextField), 'again');
    await tester.pump(const Duration(milliseconds: 300));
    await tester.enterText(find.byType(TextField), '');
    await tester.pump();
    await tester.enterText(find.byType(TextField), 'again');
    await tester.pump(const Duration(milliseconds: 300));
    expect(session.searches, hasLength(2));
    expect(actions.searches, hasLength(2));
    session.searches.removeAt(0).complete(const []);
    actions.searches.removeAt(0).complete(const []);
    await tester.pump();
    expect(spinner, findsOneWidget);
    session.searches.removeAt(0).complete(const []);
    actions.searches.removeAt(0).complete(const []);
    await tester.pump();
    expect(spinner, findsNothing);

    // The Channels tab says it has no results even when People has some.
    await tester.enterText(find.byType(TextField), 'ada');
    await tester.pump(const Duration(milliseconds: 300));
    session.searches.removeAt(0).complete(const []);
    actions.searches.removeAt(0).complete(const [
      DirectoryUser(pubkey: 'ada', displayName: 'Ada'),
    ]);
    await tester.pump();
    expect(find.text("No results for 'ada'"), findsNothing);
    await tester.tap(find.text('Channels'));
    await tester.pump();
    expect(find.text("No results for 'ada'"), findsOneWidget);
  });

  testWidgets('a failed directory offers a retry instead of no results', (
    tester,
  ) async {
    final session = _HeldSession();
    final actions = _HeldUserSearch();
    final channels = _DirectoryChannels();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          relayConfigProvider.overrideWith(_Community.new),
          relaySessionProvider.overrideWith(() => session),
          channelActionsProvider.overrideWith((ref) => actions),
          channelsProvider.overrideWith(() => channels),
          recentSearchesProvider.overrideWith(_NoRecentSearches.new),
          profileProvider.overrideWith(_Profile.new),
        ],
        child: const SearchPage(),
      ),
    );
    await tester.pumpAndSettle();

    final spinner = find.byType(BuzzLoadingIndicator);
    final failed = find.byKey(const Key('search-channels-failed'));
    final retry = find.widgetWithText(TextButton, 'Retry channels');
    final unjoined = find.byKey(
      const ValueKey('search-channel-row-4things-id'),
    );

    // A bare "#" lists joined channels, like desktop. The directory finishing
    // last must still stop the spinner.
    await tester.enterText(find.byType(TextField), '#');
    await tester.pump(const Duration(milliseconds: 300));
    session.searches.removeAt(0).complete(const []);
    actions.searches.removeAt(0).complete(const []);
    await tester.pump();
    expect(spinner, findsOneWidget);
    channels.failDirectory();
    await tester.pump();
    expect(spinner, findsNothing);
    expect(
      find.byKey(const ValueKey('search-channel-row-general-id')),
      findsOneWidget,
    );
    expect(failed, findsOneWidget);

    // Each query tries the directory again. A failure must not read as
    // "No results".
    await tester.enterText(find.byType(TextField), 'zzzz');
    await tester.pump(const Duration(milliseconds: 300));
    expect(channels.directoryRequests, 2);
    session.searches.removeAt(0).complete(const []);
    actions.searches.removeAt(0).complete(const []);
    channels.failDirectory();
    await tester.pump();
    expect(spinner, findsNothing);
    expect(failed, findsOneWidget);
    expect(find.text("No results for 'zzzz'"), findsNothing);

    // When it fails again, the member match stays beside the notice.
    await tester.enterText(find.byType(TextField), '4things');
    await tester.pump(const Duration(milliseconds: 300));
    expect(channels.directoryRequests, 3);
    session.searches.removeAt(0).complete(const []);
    actions.searches.removeAt(0).complete(const []);
    channels.failDirectory();
    await tester.pump();
    expect(spinner, findsNothing);
    expect(
      find.byKey(const ValueKey('search-channel-row-team-4things-id')),
      findsOneWidget,
    );
    expect(unjoined, findsNothing);
    expect(failed, findsOneWidget);

    // Retry loads the directory and finds the open channel.
    await tester.tap(retry);
    await tester.pump();
    expect(channels.directoryRequests, 4);
    expect(spinner, findsOneWidget);
    expect(failed, findsNothing);
    channels.finishDirectory();
    await tester.pump();
    expect(spinner, findsNothing);
    expect(failed, findsNothing);
    expect(unjoined, findsOneWidget);

    // A community switch retires a directory load still in flight. When it
    // finishes, it must not settle the new community's search.
    final community =
        ProviderScope.containerOf(
              tester.element(find.byType(SearchPage)),
            ).read(relayConfigProvider.notifier)
            as _Community;
    community.switchTo('http://other.test');
    await tester.pump();
    await tester.enterText(find.byType(TextField), 'ops');
    await tester.pump(const Duration(milliseconds: 300));
    expect(channels.directoryRequests, 5);
    community.switchTo('http://third.test');
    await tester.pump();
    await tester.enterText(find.byType(TextField), 'dev');
    await tester.pump(const Duration(milliseconds: 300));
    expect(channels.directoryRequests, 6);
    session.searches.removeLast().complete(const []);
    actions.searches.removeLast().complete(const []);
    channels.finishDirectory();
    await tester.pump();
    expect(spinner, findsOneWidget);
    channels.finishDirectory();
    await tester.pump();
    expect(spinner, findsNothing);
  });

  testWidgets('the production directory refresh holds the spinner', (
    tester,
  ) async {
    final session = _MembershipHeldSession();
    final actions = _HeldUserSearch();

    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          relayConfigProvider.overrideWith(_Community.new),
          myPubkeyProvider.overrideWith((ref) => 'test'),
          relaySessionProvider.overrideWith(() => session),
          channelActionsProvider.overrideWith((ref) => actions),
          channelsProvider.overrideWith(_NoMemberChannels.new),
          recentSearchesProvider.overrideWith(_NoRecentSearches.new),
          profileProvider.overrideWith(_Profile.new),
        ],
        child: const SearchPage(),
      ),
    );
    await tester.pumpAndSettle();

    final spinner = find.byType(BuzzLoadingIndicator);

    // The real refresh reads memberships before it marks the directory
    // loading. Messages and people finishing first must not stop the spinner.
    await tester.enterText(find.byType(TextField), '4things');
    await tester.pump(const Duration(milliseconds: 300));
    session.searches.removeAt(0).complete(const []);
    actions.searches.removeAt(0).complete(const []);
    await tester.pump();
    expect(session.memberships, hasLength(1));
    expect(spinner, findsOneWidget);
    expect(find.text("No results for '4things'"), findsNothing);

    // The refresh fails; search says so and offers a retry.
    session.memberships.single.completeError(StateError('relay down'));
    await tester.pump();
    expect(spinner, findsNothing);
    expect(find.byKey(const Key('search-channels-failed')), findsOneWidget);

    // Another caller's directory load keeps the spinner on until it ends,
    // though this search's own request returns at once.
    final container = ProviderScope.containerOf(
      tester.element(find.byType(SearchPage)),
    );
    final status = container.read(channelDirectoryLoadStatusProvider.notifier);
    final scope = channelDirectoryScope('http://community.test', 'test');
    status.markLoading(scope);
    await tester.enterText(find.byType(TextField), 'ops');
    await tester.pump(const Duration(milliseconds: 300));
    session.searches.removeAt(0).complete(const []);
    actions.searches.removeAt(0).complete(const []);
    await tester.pump();
    expect(session.memberships, hasLength(1));
    expect(spinner, findsOneWidget);
    status.markError(scope);
    await tester.pump();
    expect(spinner, findsNothing);
  });
}

Channel _channel(String name, {required bool isMember}) => Channel(
  id: '$name-id',
  name: name,
  channelType: 'stream',
  visibility: 'open',
  description: '',
  memberCount: 3,
  createdBy: 'test',
  createdAt: DateTime(2025),
  isMember: isMember,
);

class _Community extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(baseUrl: 'http://community.test');

  void switchTo(String baseUrl) => state = RelayConfig(baseUrl: baseUrl);
}

/// Holds every message search open until the test answers it.
class _HeldSession extends RelaySessionNotifier {
  final searches = <Completer<List<NostrEvent>>>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    final search = Completer<List<NostrEvent>>();
    searches.add(search);
    return search.future;
  }
}

/// Holds every people search open until the test answers it.
class _HeldUserSearch implements ChannelActions {
  final searches = <Completer<List<DirectoryUser>>>[];

  @override
  Future<List<DirectoryUser>> searchUsers(String query, {int limit = 8}) {
    final search = Completer<List<DirectoryUser>>();
    searches.add(search);
    return search.future;
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

/// Lists member channels, and adds the open directory when it is loaded.
class _DirectoryChannels extends ChannelsNotifier {
  final _directories = <Completer<bool>>[];
  var directoryRequests = 0;
  Duration? maxAge;

  void finishDirectory() => _directories.removeAt(0).complete(true);

  void failDirectory() => _directories.removeAt(0).complete(false);

  @override
  Future<List<Channel>> build() async => [
    _channel('general', isMember: true),
    _channel('team-4things', isMember: true),
  ];

  @override
  Future<void> ensureDirectoryLoaded({Duration? maxAge}) async {
    this.maxAge = maxAge;
    final status = ref.read(channelDirectoryLoadStatusProvider.notifier);
    final scope = channelDirectoryScope(
      ref.read(relayConfigProvider).baseUrl,
      ref.read(myPubkeyProvider),
    );
    final directory = ref.read(channelDirectoryLoadStatusProvider);
    if (directory.scope == scope &&
        directory.status == ChannelDirectoryLoadStatus.loaded) {
      return;
    }
    directoryRequests++;
    // Like production, the status turns loading only after memberships
    // arrive, so it stays as it was while the request is held.
    final load = Completer<bool>();
    _directories.add(load);
    if (!await load.future) {
      // Like production retryDirectory: record the error, do not throw.
      status.markError(scope);
      return;
    }
    // Production marks the directory loaded before it installs the list.
    status.markLoaded(scope);
    state = AsyncData([...?state.value, _channel('4things', isMember: false)]);
  }
}

class _NoRecentSearches extends RecentSearchesNotifier {
  @override
  List<String> build() => const [];
}

class _Profile extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async =>
      const UserProfile(pubkey: 'test', displayName: 'Test');
}

/// Real channel loading, starting with no memberships.
class _NoMemberChannels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [];
}

/// Holds the membership query that starts a real directory refresh.
class _MembershipHeldSession extends _HeldSession {
  final memberships = <Completer<List<NostrEvent>>>[];

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    final membership = Completer<List<NostrEvent>>();
    memberships.add(membership);
    return membership.future;
  }
}
