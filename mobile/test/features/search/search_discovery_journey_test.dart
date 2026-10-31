import 'dart:async';

import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/search/search_page.dart';
import 'package:buzz/features/search/search_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/buzz_loading_indicator.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

// Separate from the presentation tests: this journey keeps the search and
// channel notifiers real and replaces only storage and the relay boundary.
void main() {
  testWidgets(
    'search discovers channels, retries and retires old community work',
    (tester) async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final session = _Relay();
      final container = ProviderContainer(
        retry: (_, _) => null,
        overrides: [
          relaySessionProvider.overrideWith(() => session),
          myPubkeyProvider.overrideWithValue('me'),
          savedPrefsProvider.overrideWithValue(prefs),
        ],
      );
      addTearDown(container.dispose);
      await tester.pumpWidget(
        UncontrolledProviderScope(
          container: container,
          child: MaterialApp(theme: AppTheme.light(), home: const SearchPage()),
        ),
      );
      await tester.pumpAndSettle();
      expect(container.read(channelsProvider).requireValue, isEmpty);
      expect(session.directoryRequests, 0);

      await tester.enterText(find.byType(TextField), 'welcome');
      await tester.pump(const Duration(milliseconds: 310));
      await tester.pump();
      expect(find.byType(BuzzLoadingIndicator), findsOneWidget);
      expect(find.byKey(const Key('search-no-results-state')), findsNothing);

      // A second query shares the pending directory request; neither fast
      // message nor people results may incorrectly stop its loading indicator.
      await tester.enterText(find.byType(TextField), 'wel');
      await tester.pump(const Duration(milliseconds: 310));
      await tester.pump();
      expect(session.directoryRequests, 1);
      expect(find.byType(BuzzLoadingIndicator), findsOneWidget);
      session.directory.completeError(StateError('offline'));
      await tester.pumpAndSettle();
      expect(find.text('Couldn’t load channels.'), findsOneWidget);
      expect(find.text('Retry'), findsOneWidget);
      expect(find.byKey(const Key('search-no-results-state')), findsNothing);

      session.directory = Completer<List<NostrEvent>>();
      await tester.tap(find.text('Retry'));
      await tester.pump(const Duration(milliseconds: 310));
      await tester.pump();
      session.directory.complete([
        _meta('open', 'welcome-everyone'),
        _meta('private', 'welcome-staff', visibility: 'private'),
        _meta('dm', 'welcome-dm', type: 'dm'),
      ]);
      await tester.pumpAndSettle();
      expect(find.text('welcome-everyone'), findsOneWidget);
      expect(find.text('welcome-staff'), findsNothing);
      expect(find.text('welcome-dm'), findsNothing);
      expect(find.text('Open'), findsOneWidget);
      expect(find.text('Retry'), findsNothing);
      expect(find.byType(BuzzLoadingIndicator), findsNothing);
      expect(
        container.read(searchProvider).channelResults.single.isMember,
        isFalse,
      );

      // Reuse the loaded directory while editing, including a genuine empty hit.
      final requests = session.directoryRequests;
      await tester.enterText(find.byType(TextField), 'absent');
      await tester.pump(const Duration(milliseconds: 310));
      await tester.pumpAndSettle();
      expect(find.text("No results for 'absent'"), findsOneWidget);
      expect(session.directoryRequests, requests);

      // Same text in a new community must not accept an old message response.
      session.messages = Completer<List<NostrEvent>>();
      await tester.enterText(find.byType(TextField), 'welcome');
      await tester.pump(const Duration(milliseconds: 310));
      await tester.pump();
      final oldMessages = session.messages!;
      session.messages = null;
      session.directory = Completer<List<NostrEvent>>();
      session.memberships = Completer<List<NostrEvent>>();
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://other.example');
      await tester.pump();
      await tester.enterText(find.byType(TextField), '');
      await tester.enterText(find.byType(TextField), 'welcome');
      await tester.pump(const Duration(milliseconds: 310));
      await tester.pump();
      expect(find.text('welcome-everyone'), findsNothing);
      session.memberships!.complete(const []);
      oldMessages.complete([
        const NostrEvent(
          id: 'old-message',
          pubkey: 'other',
          createdAt: 1,
          kind: 9,
          tags: [],
          content: 'old community secret',
          sig: '',
        ),
      ]);
      session.directory.complete([_meta('new', 'welcome-new-community')]);
      await tester.pumpAndSettle();
      expect(find.text('welcome-new-community'), findsOneWidget);
      expect(find.text('welcome-everyone'), findsNothing);
      expect(container.read(searchProvider).messageResults, isEmpty);
      expect(tester.takeException(), isNull);
      await tester.pumpWidget(const SizedBox.shrink());
      container.dispose();
    },
  );
}

NostrEvent _meta(
  String id,
  String name, {
  String visibility = 'open',
  String type = 'stream',
}) => NostrEvent(
  id: 'meta-$id',
  pubkey: 'creator',
  createdAt: 1,
  kind: 39000,
  tags: [
    ['d', id],
    ['name', name],
    ['t', type],
    [visibility == 'private' ? 'private' : 'public'],
  ],
  content: '',
  sig: '',
);

class _Relay extends RelaySessionNotifier {
  var directory = Completer<List<NostrEvent>>();
  Completer<List<NostrEvent>>? messages;
  Completer<List<NostrEvent>>? memberships;
  int directoryRequests = 0;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filter.search != null && filter.kinds.contains(9)) {
      return messages?.future ?? Future.value(const []);
    }
    return const [];
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final filter = filters.single;
    if (filter.kinds.contains(39002)) {
      return memberships?.future ?? Future.value(const []);
    }
    if (filter.kinds.contains(39000) && filter.tags['#d'] == null) {
      if (filter.until != null) return const [];
      directoryRequests++;
      return directory.future;
    }
    return const [];
  }

  @override
  Future<List<NostrEvent>> searchProfiles(
    String query, {
    int limit = 50,
    Duration timeout = const Duration(seconds: 8),
  }) async => const [];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String)? onClosed,
  }) async => () {};
}
