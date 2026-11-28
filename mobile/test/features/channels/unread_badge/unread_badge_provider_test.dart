import 'dart:async';

import 'package:buzz/features/activity/activity_provider.dart';
import 'package:buzz/features/activity/feed_item.dart';
import 'package:buzz/features/activity/inbox_local_state_provider.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_mutes/channel_mutes_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/unread_badge/observed_unread_event.dart';
import 'package:buzz/features/channels/unread_badge/unread_badge_provider.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../../helpers/thread_ids.dart';

const _relayA = 'https://a.example.org';
const _relayB = 'https://b.example.org';

Channel _channel({
  String id = 'ch-a',
  String type = 'stream',
  bool member = true,
  bool archived = false,
}) => Channel(
  id: id,
  name: id,
  channelType: type,
  visibility: 'open',
  description: '',
  createdBy: 'creator',
  createdAt: DateTime.utc(2024),
  memberCount: 1,
  isMember: member,
  archivedAt: archived ? DateTime.utc(2024) : null,
);

FeedItem _item({
  String id = 'mention',
  String? channelId = 'ch-a',
  int kind = 9,
  String category = 'mention',
  List<List<String>> tags = const [],
}) => FeedItem(
  id: id,
  kind: kind,
  pubkey: 'sender',
  content: 'An update',
  createdAt: 100,
  channelId: channelId,
  channelName: channelId ?? '',
  tags: tags,
  category: category,
);

HomeFeedResponse _feed(List<FeedItem> items, {bool complete = true}) =>
    HomeFeedResponse(
      mentions: items.where((item) => item.category == 'mention').toList(),
      needsAction: items
          .where((item) => item.category == 'needs_action')
          .toList(),
      activity: items.where((item) => item.category == 'activity').toList(),
      agentActivity: items
          .where((item) => item.category == 'agent_activity')
          .toList(),
      complete: complete,
    );

class _Fixture {
  _Fixture({
    List<Channel>? channels,
    List<FeedItem> items = const [],
    Map<String, int> contexts = const {},
    Map<String, String> forced = const {},
    bool readReady = true,
    ChannelsNotifier? channelNotifier,
    ActivityNotifier? activity,
    RelaySessionNotifier? session,
    _AuthNotifier? auth,
  }) {
    container = ProviderContainer(
      retry: (_, _) => null,
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        relayConfigProvider.overrideWith(_RelayConfigNotifier.new),
        myPubkeyProvider.overrideWithValue('reader'),
        if (session != null) relaySessionProvider.overrideWith(() => session),
        authProvider.overrideWith(() => auth ?? _AuthNotifier()),
        channelsProvider.overrideWith(
          () =>
              channelNotifier ??
              _StubbedChannelsNotifier(channels ?? [_channel()]),
        ),
        activityProvider.overrideWith(
          () => activity ?? _ActivityNotifier((_) async => _feed(items)),
        ),
        readStateProvider.overrideWith(
          () => _ReadStateNotifier(
            ReadStateState(
              isReady: readReady,
              pubkey: 'reader',
              contexts: contexts,
              version: 1,
              forcedUnreadContexts: forced,
            ),
          ),
        ),
        channelMutesProvider.overrideWith(_MutedChannels.new),
      ],
    );
    addTearDown(container.dispose);
  }

  static late SharedPreferences prefs;
  late final ProviderContainer container;

  Future<void> load() async {
    await container.read(authProvider.future);
    await container.read(channelsProvider.future);
    await container.read(activityProvider.future);
  }

  UnreadBadge get badge => container.read(unreadBadgeProvider);
  InboxLocalStateNotifier get local =>
      container.read(inboxLocalStateProvider.notifier);
}

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    _Fixture.prefs = await SharedPreferences.getInstance();
  });

  test('everything in Activity read and no reminders gives clear', () async {
    final fixture = _Fixture(items: [_item()], contexts: {'ch-a': 100});
    await fixture.load();
    expect(fixture.badge, UnreadBadge.clear);
  });

  test('empty Activity and no reminders gives clear', () async {
    final fixture = _Fixture();
    await fixture.load();
    expect(fixture.badge, UnreadBadge.clear);
  });

  for (final (name, item, channel) in [
    ('DM', _item(category: 'activity'), _channel(type: 'dm')),
    ('mention', _item(), _channel()),
    ('agent event', _item(kind: 43004, category: 'agent_activity'), _channel()),
    (
      'followed thread reply',
      _item(
        category: 'activity',
        tags: [
          ['e', threadIdRoot, '', 'root'],
        ],
      ),
      _channel(),
    ),
  ]) {
    test('one unread $name gives unread', () async {
      final fixture = _Fixture(channels: [channel], items: [item]);
      await fixture.load();
      expect(fixture.badge, UnreadBadge.unread);
      (fixture.container.read(readStateProvider.notifier) as _ReadStateNotifier)
          .setState(
            const ReadStateState(
              isReady: true,
              pubkey: 'reader',
              contexts: {'ch-a': 100},
              version: 2,
            ),
          );
      expect(fixture.badge, UnreadBadge.clear);
    });
  }

  test(
    'an unread approval with no channel gives unread, and clears when done',
    () async {
      final fixture = _Fixture(
        items: [
          _item(
            id: 'approval',
            channelId: null,
            kind: 46010,
            category: 'needs_action',
          ),
        ],
      );
      await fixture.load();
      expect(fixture.badge, UnreadBadge.unread);
      fixture.local.markDone('approval');
      expect(fixture.badge, UnreadBadge.clear);
    },
  );

  for (final (name, context) in [('channel', 'ch-a'), ('message', 'msg:abc')]) {
    test('a $name marked unread alone gives unread', () async {
      final fixture = _Fixture(forced: {context: 'ch-a'});
      await fixture.load();
      // The channel is muted, but an explicit mark-unread still counts.
      expect(
        fixture.container.read(channelMutesProvider).confirmedMutedChannelIds,
        contains('ch-a'),
      );
      expect(fixture.badge, UnreadBadge.unread);
    });
  }

  test('an Activity row marked unread after reading gives unread', () async {
    final fixture = _Fixture(items: [_item()], contexts: {'ch-a': 100});
    await fixture.load();
    expect(fixture.badge, UnreadBadge.clear);
    fixture.local.markUnread(['mention'], baseline: 100);
    expect(fixture.badge, UnreadBadge.unread);
    fixture.local.clearUnread(['mention']);
    expect(fixture.badge, UnreadBadge.clear);
  });

  for (final broadcast in [false, true]) {
    test(
      broadcast
          ? 'a broadcast with no mention alone gives clear'
          : 'a plain channel message alone gives clear',
      () async {
        final event = makeObservedUnreadEvent(
          id: 'ordinary',
          createdAt: 100,
          rootId: broadcast ? threadIdRoot : null,
          highPriority: false,
          channelType: 'stream',
          isThreadedReply: broadcast,
          isBroadcast: broadcast,
        );
        final fixture = _Fixture(
          channelNotifier: _StubbedChannelsNotifier(
            [_channel()],
            observed: {
              'ch-a': {'ordinary': event},
            },
          ),
        );
        await fixture.load();
        expect(fixture.badge, UnreadBadge.clear);
      },
    );
  }

  test(
    'a mention in Activity gives unread even if the channel is muted',
    () async {
      final fixture = _Fixture(items: [_item()]);
      await fixture.load();
      expect(
        fixture.container.read(channelMutesProvider).confirmedMutedChannelIds,
        contains('ch-a'),
      );
      expect(fixture.badge, UnreadBadge.unread);
    },
  );

  test(
    'a mark-unread flag on an archived or non-member channel is ignored',
    () async {
      for (final member in [false, true]) {
        for (final archived in [false, true]) {
          for (final forced in [false, true]) {
            final fixture = _Fixture(
              channels: [_channel(member: member, archived: archived)],
              forced: forced ? {'ch-a': 'ch-a'} : {},
            );
            await fixture.load();
            expect(
              fixture.badge,
              member && !archived && forced
                  ? UnreadBadge.unread
                  : UnreadBadge.clear,
              reason: 'member=$member, archived=$archived, forced=$forced',
            );
          }
        }
      }
    },
  );

  test('an unread item in another community gives clear', () async {
    final nextFeed = Completer<HomeFeedResponse>();
    final fixture = _Fixture(
      activity: _ActivityNotifier(
        (url) async => url == _relayA ? _feed([_item()]) : nextFeed.future,
      ),
    );
    final values = <UnreadBadge>[];
    fixture.container.listen(
      unreadBadgeProvider,
      (_, next) => values.add(next),
      fireImmediately: true,
    );
    await fixture.load();
    expect(fixture.badge, UnreadBadge.unread);
    fixture.container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: _relayB);
    expect(fixture.badge, UnreadBadge.pending);
    // Riverpod retains the old feed's value during this dependency reload.
    expect(fixture.container.read(activityProvider).value!.all, isNotEmpty);
    nextFeed.complete(_feed([]));
    await fixture.container.read(activityProvider.future);
    expect(fixture.badge, UnreadBadge.clear);
    expect(values, [
      UnreadBadge.pending,
      UnreadBadge.unread,
      UnreadBadge.pending,
      UnreadBadge.clear,
    ]);
  });

  test('a local mark unread from another community does not count', () async {
    final fixture = _Fixture(items: [_item()], contexts: {'ch-a': 100});
    await fixture.load();
    fixture.local.markUnread(['mention'], baseline: 100);
    expect(fixture.badge, UnreadBadge.unread);
    fixture.container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: _relayB);
    await fixture.container.read(activityProvider.future);
    expect(
      fixture.container.read(inboxLocalStateProvider).unreadBaselines,
      isEmpty,
    );
    expect(fixture.badge, UnreadBadge.clear);
    fixture.container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: _relayA);
    await fixture.container.read(activityProvider.future);
    expect(fixture.badge, UnreadBadge.unread);
  });

  test('pending while read state is not ready', () async {
    final fixture = _Fixture(items: [_item()], readReady: false);
    await fixture.load();
    expect(fixture.badge, UnreadBadge.pending);
  });

  test('pending while Activity has not loaded', () async {
    final fixture = _Fixture(
      activity: _ActivityNotifier((_) => Completer<HomeFeedResponse>().future),
    );
    await fixture.container.read(authProvider.future);
    await fixture.container.read(channelsProvider.future);
    expect(fixture.badge, UnreadBadge.pending);
  });

  test('pending while channels are loading', () async {
    final fixture = _Fixture(channelNotifier: _LoadingChannelsNotifier());
    await fixture.container.read(authProvider.future);
    await fixture.container.read(activityProvider.future);
    expect(fixture.badge, UnreadBadge.pending);
  });

  test('pending while auth is unknown or loading', () async {
    final fixture = _Fixture(auth: _AuthNotifier(status: AuthStatus.unknown));
    expect(fixture.badge, UnreadBadge.pending);
    await fixture.container.read(authProvider.future);
    expect(fixture.badge, UnreadBadge.pending);
  });

  test('signed out gives clear', () async {
    final fixture = _Fixture(
      auth: _AuthNotifier(status: AuthStatus.unauthenticated),
      activity: _ActivityNotifier(
        (_) => throw StateError('Activity must not start'),
      ),
      channelNotifier: _UnrequestedChannelsNotifier(),
    );
    await fixture.container.read(authProvider.future);
    expect(fixture.badge, UnreadBadge.clear);
    expect(fixture.container.exists(activityProvider), isFalse);
    expect(fixture.container.exists(channelsProvider), isFalse);
  });

  test('pending then data does not pass through clear', () async {
    final feed = Completer<HomeFeedResponse>();
    final fixture = _Fixture(activity: _ActivityNotifier((_) => feed.future));
    final values = <UnreadBadge>[];
    fixture.container.listen(
      unreadBadgeProvider,
      (_, next) => values.add(next),
      fireImmediately: true,
    );
    await fixture.container.read(authProvider.future);
    await fixture.container.read(channelsProvider.future);
    expect(fixture.badge, UnreadBadge.pending);
    expect(values, [UnreadBadge.pending]);
    feed.complete(_feed([_item()]));
    await fixture.container.read(activityProvider.future);
    expect(fixture.badge, UnreadBadge.unread);
    expect(values, [UnreadBadge.pending, UnreadBadge.unread]);
  });

  for (final source in ['auth', 'Activity', 'channels']) {
    test('pending during a $source refresh even with retained data', () async {
      final refreshed = Completer<void>();
      var loaded = false;
      Future<void> gateReload() async {
        if (loaded) await refreshed.future;
        loaded = true;
      }

      final fixture = _Fixture(
        auth: _AuthNotifier(beforeLoad: source == 'auth' ? gateReload : null),
        channelNotifier: _StubbedChannelsNotifier([
          _channel(),
        ], beforeLoad: source == 'channels' ? gateReload : null),
        activity: _ActivityNotifier((_) async {
          if (source == 'Activity') await gateReload();
          return _feed([_item()]);
        }),
      );
      await fixture.load();
      expect(fixture.badge, UnreadBadge.unread);
      switch (source) {
        case 'auth':
          fixture.container.invalidate(authProvider);
          expect(fixture.container.read(authProvider).asData, isNotNull);
        case 'Activity':
          fixture.container.invalidate(activityProvider);
          expect(fixture.container.read(activityProvider).asData, isNotNull);
        case 'channels':
          fixture.container.invalidate(channelsProvider);
          expect(fixture.container.read(channelsProvider).asData, isNotNull);
      }
      expect(fixture.badge, UnreadBadge.pending);
      refreshed.complete();
      await fixture.load();
      expect(fixture.badge, UnreadBadge.unread);
    });
  }

  for (final source in ['auth', 'Activity', 'channels']) {
    test('pending when $source fails to load', () async {
      final fixture = _Fixture();
      await fixture.load();
      expect(fixture.badge, UnreadBadge.clear);
      final error = StateError('injected load failure');
      switch (source) {
        case 'auth':
          (fixture.container.read(authProvider.notifier) as _AuthNotifier).fail(
            error,
          );
        case 'Activity':
          (fixture.container.read(activityProvider.notifier)
                  as _ActivityNotifier)
              .fail(error);
        case 'channels':
          (fixture.container.read(channelsProvider.notifier)
                  as _StubbedChannelsNotifier)
              .fail(error);
      }
      expect(fixture.badge, UnreadBadge.pending);
    });
  }

  test(
    'pending when the real Activity read fails on both transports',
    () async {
      final session = _HistorySession([
        NostrEvent(
          id: 'mention',
          pubkey: 'sender',
          createdAt: 100,
          kind: 9,
          tags: const [
            ['h', 'ch-a'],
            ['p', 'reader'],
          ],
          content: 'An update',
          sig: '',
        ),
      ]);
      final fixture = _Fixture(activity: ActivityNotifier(), session: session);
      await fixture.load();
      expect(fixture.badge, UnreadBadge.unread);

      // The notifier does not report an error: it publishes a feed without the
      // sources it could not read, which must not clear the icon.
      session.fail = true;
      fixture.container.invalidate(activityProvider);
      final feed = await fixture.container.read(activityProvider.future);
      expect(feed.isEmpty, isTrue);
      expect(fixture.badge, UnreadBadge.pending);

      session.fail = false;
      fixture.container.invalidate(activityProvider);
      await fixture.container.read(activityProvider.future);
      expect(fixture.badge, UnreadBadge.unread);
    },
  );

  group('while an Activity source is missing', () {
    test('an unread row raises a clear icon', () async {
      var feed = _feed(const []);
      final fixture = _Fixture(activity: _ActivityNotifier((_) async => feed));
      await fixture.load();
      expect(fixture.badge, UnreadBadge.clear);

      feed = _feed([_item()], complete: false);
      fixture.container.invalidate(activityProvider);
      await fixture.container.read(activityProvider.future);
      expect(fixture.badge, UnreadBadge.unread);
    });

    for (final context in ['ch-a', 'msg:abc']) {
      test('a mark-unread flag ($context) raises a clear icon', () async {
        var feed = _feed(const []);
        final fixture = _Fixture(
          activity: _ActivityNotifier((_) async => feed),
        );
        await fixture.load();
        expect(fixture.badge, UnreadBadge.clear);

        feed = _feed(const [], complete: false);
        fixture.container.invalidate(activityProvider);
        await fixture.container.read(activityProvider.future);
        expect(fixture.badge, UnreadBadge.pending);

        (fixture.container.read(readStateProvider.notifier)
                as _ReadStateNotifier)
            .setState(
              ReadStateState(
                isReady: true,
                pubkey: 'reader',
                contexts: const {},
                version: 2,
                forcedUnreadContexts: {context: 'ch-a'},
              ),
            );
        expect(fixture.badge, UnreadBadge.unread);
      });
    }

    test('nothing known unread holds rather than clears', () async {
      final fixture = _Fixture(
        activity: _ActivityNotifier(
          (_) async => _feed([_item()], complete: false),
        ),
        contexts: {'ch-a': 100},
      );
      await fixture.load();
      expect(fixture.badge, UnreadBadge.pending);
    });
  });

  test('platformCount maps pending to null, clear to 0, unread to 1', () {
    expect(UnreadBadge.pending.platformCount, isNull);
    expect(UnreadBadge.clear.platformCount, 0);
    expect(UnreadBadge.unread.platformCount, 1);
  });
}

class _StubbedChannelsNotifier extends ChannelsNotifier {
  _StubbedChannelsNotifier(
    this.channels, {
    this.observed = const {},
    this.beforeLoad,
  });
  final Future<void> Function()? beforeLoad;
  final List<Channel> channels;
  final Map<String, Map<String, ObservedUnreadEvent>> observed;

  @override
  Future<List<Channel>> build() async {
    await beforeLoad?.call();
    return channels;
  }

  void fail(Object error) => state = AsyncError(error, StackTrace.current);

  @override
  Map<String, int> get latestObservedByChannel => {
    for (final id in observed.keys) id: 100,
  };

  @override
  Map<String, Map<String, ObservedUnreadEvent>>
  get observedUnreadEventsByChannel => observed;
}

class _LoadingChannelsNotifier extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() => Completer<List<Channel>>().future;
}

class _UnrequestedChannelsNotifier extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() => throw StateError('Channels must not start');
}

class _ReadStateNotifier extends ReadStateNotifier {
  _ReadStateNotifier(this.initial);
  final ReadStateState initial;
  @override
  ReadStateState build() => initial;
  void setState(ReadStateState next) => state = next;
}

class _ActivityNotifier extends ActivityNotifier {
  _ActivityNotifier(this.load);
  final Future<HomeFeedResponse> Function(String relayUrl) load;
  @override
  Future<HomeFeedResponse> build() =>
      load(ref.watch(relayConfigProvider).baseUrl);

  void fail(Object error) => state = AsyncError(error, StackTrace.current);
}

/// A connected relay that answers Activity history from [events] until it
/// fails every read.
class _HistorySession extends RelaySessionNotifier {
  _HistorySession(this.events);
  final List<NostrEvent> events;
  bool fail = false;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (fail) throw StateError('relay unavailable');
    return [for (final filter in filters) ...events.where(_matcher(filter))];
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (fail) throw StateError('relay unavailable');
    return events.where(_matcher(filter)).toList();
  }

  static bool Function(NostrEvent) _matcher(NostrFilter filter) =>
      (event) =>
          filter.kinds.contains(event.kind) &&
          filter.tags.entries.every(
            (entry) => event.tags.any(
              (tag) =>
                  tag.length > 1 &&
                  '#${tag[0]}' == entry.key &&
                  entry.value.contains(tag[1]),
            ),
          );

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async => () {};
}

class _AuthNotifier extends AuthNotifier {
  _AuthNotifier({this.status = AuthStatus.authenticated, this.beforeLoad});
  final AuthStatus status;
  final Future<void> Function()? beforeLoad;
  @override
  Future<AuthState> build() async {
    await beforeLoad?.call();
    return AuthState(status: status);
  }

  void fail(Object error) => state = AsyncError(error, StackTrace.current);
}

class _RelayConfigNotifier extends RelayConfigNotifier {
  @override
  RelayConfig build() => const RelayConfig(baseUrl: _relayA);
}

class _MutedChannels extends ChannelMutesNotifier {
  @override
  ChannelMutesState build() => const ChannelMutesState(
    isReady: true,
    confirmedMutedChannelIds: {'ch-a'},
  );
}
