import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/search/recent_searches_provider.dart';
import 'package:buzz/features/search/search_page.dart';
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
    expect(spinner, findsOneWidget);
    expect(unjoined, findsNothing);

    channels.finishDirectory();
    await tester.pump();
    expect(unjoined, findsOneWidget);
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

    // A one-letter typo still finds the member channel, like desktop.
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
  final _directory = Completer<void>();
  var directoryRequests = 0;

  void finishDirectory() => _directory.complete();

  @override
  Future<List<Channel>> build() async => [_channel('general', isMember: true)];

  @override
  Future<void> ensureDirectoryLoaded() async {
    final status = ref.read(channelDirectoryLoadStatusProvider.notifier);
    final scope = channelDirectoryScope(
      ref.read(relayConfigProvider).baseUrl,
      ref.read(myPubkeyProvider),
    );
    if (ref.read(channelDirectoryLoadStatusProvider).status ==
        ChannelDirectoryLoadStatus.loaded) {
      return;
    }
    directoryRequests++;
    status.markLoading(scope);
    await _directory.future;
    state = AsyncData([...?state.value, _channel('4things', isMember: false)]);
    status.markLoaded(scope);
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
