import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/channels/channel_mutes/channel_mutes_manager.dart';
import 'package:buzz/features/channels/channel_mutes/channel_mutes_provider.dart';
import 'package:buzz/features/channels/channel_mutes/channel_mutes_storage.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_provider.dart';
import 'package:buzz/features/channels/channel_sort/channel_sort_provider.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_provider.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_storage.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/read_state/read_state_format.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:flutter/widgets.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_manager.dart';
import 'package:buzz/features/channels/channel_sort/channel_sort_manager.dart';
import 'package:buzz/features/channels/channel_sort/channel_sort_storage.dart';
import 'package:buzz/features/channels/channel_stars/channel_stars_manager.dart';
import 'package:fake_async/fake_async.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'sidebar_sync_fixture.dart';

/// The per-entry lanes share one manager shape; each case runs on both.
class _Lane {
  const _Lane(this.dTag, this.field, this.build);

  final String dTag;
  final String field;
  final _Subject Function(SharedPreferences prefs, SidebarRelay relay) build;
}

class _Subject {
  _Subject({
    required this.init,
    required this.values,
    required this.set,
    required this.refresh,
    required this.dispose,
  });

  final Future<void> Function() init;
  final Map<String, bool> Function() values;
  final void Function(String channel, bool value) set;
  final void Function() refresh;
  final void Function() dispose;
}

final _lanes = [
  _Lane('channel-stars', 'starred', (prefs, relay) {
    final m = ChannelStarsManager(
      pubkey: relay.pubkey,
      relayUrl: 'https://relay.example',
      prefs: prefs,
      crypto: ChannelStarsCrypto(relay.keys.nsec, relay.pubkey),
      relaySession: relay.session,
      signedEventRelay: relay.signer,
      remoteEnabled: true,
      onChanged: () {},
    );
    return _Subject(
      init: m.initialize,
      values: () => {
        for (final e in m.store.channels.entries) e.key: e.value.starred,
      },
      set: (c, v) => v ? m.starChannel(c) : m.unstarChannel(c),
      refresh: m.refreshFromRelay,
      dispose: m.dispose,
    );
  }),
  _Lane('channel-mutes', 'muted', (prefs, relay) {
    final m = ChannelMutesManager(
      pubkey: relay.pubkey,
      relayUrl: 'https://relay.example',
      prefs: prefs,
      crypto: ChannelMutesCrypto(relay.keys.nsec, relay.pubkey),
      relaySession: relay.session,
      signedEventRelay: relay.signer,
      remoteEnabled: true,
      onChanged: () {},
    );
    return _Subject(
      init: m.initialize,
      values: () => {
        for (final e in m.store.channels.entries) e.key: e.value.muted,
      },
      set: (c, v) => v ? m.muteChannel(c) : m.unmuteChannel(c),
      refresh: m.refreshFromRelay,
      dispose: m.dispose,
    );
  }),
];

class _Lifecycle extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.paused;
  void resume() => state = AppLifecycleState.resumed;
}

class _Config extends RelayConfigNotifier {
  _Config(this.nsec);
  final String nsec;
  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: nsec);
}

class _SwitchableConfig extends RelayConfigNotifier {
  _SwitchableConfig(this.nsec);
  final String nsec;
  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example', nsec: nsec);
  void switchTo(String baseUrl) =>
      state = RelayConfig(baseUrl: baseUrl, nsec: nsec);
}

class _InitializingSortSession extends ConnectedRelaySession {
  _InitializingSortSession(this.firstSnapshot);
  final NostrEvent firstSnapshot;
  final secondSnapshot = Completer<List<NostrEvent>>();
  void Function(NostrEvent)? onEvent;
  var fetchCount = 0;

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    fetchCount++;
    return fetchCount == 1 ? [firstSnapshot] : secondSnapshot.future;
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) callback, {
    void Function(String)? onClosed,
  }) async {
    onEvent = callback;
    return () => onEvent = null;
  }
}

/// Missed heads, one per lane, each marked with `zz` so the persisted
/// cache shows whether that lane's provider re-read on resume.
const _missedHeads = {
  'channel-stars':
      '{"version":1,"channels":{"zz":{"starred":true,"updatedAt":1}}}',
  'channel-mutes':
      '{"version":1,"channels":{"zz":{"muted":true,"updatedAt":1}}}',
  'channel-sections':
      '{"version":1,"sections":[{"id":"zz","name":"zz","order":0}],"assignments":{}}',
  'channel-sort': '{"version":1,"groups":{"zz":"recent"}}',
};

void main() {
  late SharedPreferences prefs;
  setUp(() async => prefs = await freshPrefs());
  wholeBlobLanes(() => prefs);

  for (final MapEntry(key: dTag, value: head) in _missedHeads.entries) {
    fakeAsyncTest('$dTag provider re-reads a missed head on resume', (clock) {
      final relay = SidebarRelay();
      final c = ProviderContainer(
        overrides: [
          savedPrefsProvider.overrideWithValue(prefs),
          relayConfigProvider.overrideWith(() => _Config(relay.keys.nsec)),
          relaySessionProvider.overrideWith(() => relay.session),
          activeCommunityProvider.overrideWith(
            (ref) async => Community(
              id: 'c',
              name: 'c',
              relayUrl: 'wss://relay.example',
              addedAt: DateTime(2026),
            ),
          ),
          appLifecycleProvider.overrideWith(_Lifecycle.new),
        ],
      );
      for (final p in [
        channelStarsProvider,
        channelMutesProvider,
        channelSectionsProvider,
        channelSortProvider,
      ]) {
        c.listen(p, (_, _) {});
      }
      bool adopted() =>
          prefs.getKeys().any((k) => '${prefs.get(k)}'.contains('zz'));
      clock.elapse(const Duration(seconds: 1));
      expect(adopted(), isFalse);
      relay.stored.add(relay.event(dTag, jsonDecode(head), nowSeconds() + 60));
      (c.read(appLifecycleProvider.notifier) as _Lifecycle).resume();
      clock.elapse(const Duration(seconds: 1));
      expect(adopted(), isTrue);
      expect(relay.reqsFor(dTag, 'l-'), hasLength(1));
      c.dispose();
    });
  }

  fakeAsyncTest('sort readiness waits for the post-subscription snapshot', (
    clock,
  ) {
    final relay = SidebarRelay();
    final first = relay.event('channel-sort', {
      'version': 1,
      'groups': {'channels': 'alpha'},
    }, nowSeconds());
    final session = _InitializingSortSession(first);
    final c = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        relayConfigProvider.overrideWith(() => _Config(relay.keys.nsec)),
        relaySessionProvider.overrideWith(() => session),
        activeCommunityProvider.overrideWith(
          (ref) async => Community(
            id: 'c',
            name: 'c',
            relayUrl: 'wss://relay.example',
            addedAt: DateTime(2026),
          ),
        ),
        appLifecycleProvider.overrideWith(_Lifecycle.new),
      ],
    );
    c.read(activeCommunityProvider);
    clock.flushMicrotasks();
    c.listen(channelSortProvider, (_, _) {});
    clock.flushMicrotasks();
    expect(session.fetchCount, 2);
    // A live preference arrives while the second history snapshot is pending.
    final latest = relay.event('channel-sort', {
      'version': 1,
      'groups': {'channels': 'recent'},
    }, nowSeconds() + 1);
    session.onEvent!(latest);
    expect(
      c.read(channelSortProvider).sortModeFor('channels'),
      ChannelSortMode.recent,
    );
    expect(c.read(channelSortProvider).isReady, isFalse);

    session.secondSnapshot.complete([latest]);
    clock.flushMicrotasks();
    expect(c.read(channelSortProvider).isReady, isTrue);
    c.dispose();
  });

  fakeAsyncTest('groups stay with the community that owns them', (clock) {
    final relay = SidebarRelay();
    final config = _SwitchableConfig(relay.keys.nsec);
    final c = ProviderContainer(
      overrides: [
        savedPrefsProvider.overrideWithValue(prefs),
        relayConfigProvider.overrideWith(() => config),
        relaySessionProvider.overrideWith(() => relay.session),
        activeCommunityProvider.overrideWith(
          (ref) async => Community(
            id: 'c',
            name: 'c',
            relayUrl: 'wss://relay.example',
            addedAt: DateTime(2026),
          ),
        ),
        appLifecycleProvider.overrideWith(_Lifecycle.new),
      ],
    );
    List<String> names() => [
      for (final s in c.read(channelSectionsProvider).store.sections) s.name,
    ];
    c.read(relaySessionProvider);
    relay.stored.add(
      relay.event('channel-sections', {
        'version': 1,
        'sections': [
          {'id': 'studio', 'name': 'Studio', 'order': 0},
        ],
        'assignments': <String, String>{},
      }, nowSeconds()),
    );
    c.listen(channelSectionsProvider, (_, _) {});
    clock.elapse(const Duration(seconds: 1));
    expect(names(), ['Studio']);

    // The same identity in a second community that has no groups.
    config.switchTo('https://second.example');
    clock.elapse(const Duration(seconds: 1));
    expect(names(), isEmpty);

    // Returning shows the first community's cached groups before its relay
    // answers.
    final held = relay.holdHistory = Completer<void>();
    config.switchTo('https://relay.example');
    clock.flushMicrotasks();
    expect(names(), ['Studio']);
    held.complete();
    c.dispose();
  });

  // Each lane: what it shows, one local change to it, and the second
  // community's own value as its relay retains it.
  final scopedLanes =
      <
        String,
        (
          Set<String> Function(ProviderContainer),
          void Function(ProviderContainer, String),
          (String, Object) Function(String),
        )
      >{
        'channel-stars': (
          (c) => c.read(channelStarsProvider).store.channels.keys.toSet(),
          (c, id) => c.read(channelStarsProvider.notifier).starChannel(id),
          (id) => (
            'channel-stars',
            {
              'version': 1,
              'channels': {
                id: {'starred': true, 'updatedAt': nowSeconds()},
              },
            },
          ),
        ),
        'channel-mutes': (
          (c) => c.read(channelMutesProvider).store.channels.keys.toSet(),
          (c, id) => c.read(channelMutesProvider.notifier).muteChannel(id),
          (id) => (
            'channel-mutes',
            {
              'version': 1,
              'channels': {
                id: {'muted': true, 'updatedAt': nowSeconds()},
              },
            },
          ),
        ),
        'channel-sections': (
          (c) => {
            for (final s in c.read(channelSectionsProvider).store.sections)
              s.name,
          },
          (c, name) =>
              c.read(channelSectionsProvider.notifier).createSection(name),
          (name) => (
            'channel-sections',
            {
              'version': 1,
              'sections': [
                {'id': name, 'name': name, 'order': 0},
              ],
              'assignments': <String, String>{},
            },
          ),
        ),
        'read-state': (
          (c) => c.read(readStateProvider).contexts.keys.toSet(),
          (c, id) => c
              .read(readStateProvider.notifier)
              .markContextRead(id, nowSeconds()),
          (id) => (
            '$readStateDTagPrefix${'b' * 32}',
            ReadStateBlob(
              clientId: 'second-device',
              contexts: {id: nowSeconds()},
            ).toJson(),
          ),
        ),
      };
  for (final MapEntry(key: dTag, value: (values, set, secondHead))
      in scopedLanes.entries) {
    for (final reaches in ['live', 'in-flight fetch']) {
      fakeAsyncTest('$dTag stay with the community they were set in '
          '(other community reached by $reaches)', (clock) {
        final relay = SidebarRelay();
        final config = _SwitchableConfig(relay.keys.nsec);
        final c = ProviderContainer(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            relayConfigProvider.overrideWith(() => config),
            relaySessionProvider.overrideWith(() => relay.session),
            activeCommunityProvider.overrideWith((ref) async => null),
            appLifecycleProvider.overrideWith(_Lifecycle.new),
          ],
        );
        // Either the first community's subscription is live at the switch, or
        // its startup fetch is still in flight.
        final history = reaches == 'live' ? null : Completer<void>();
        relay.holdHistory = history;
        for (final p in [
          channelStarsProvider,
          channelMutesProvider,
          channelSectionsProvider,
          readStateProvider,
        ]) {
          c.listen(p, (_, _) {});
        }
        clock.elapse(const Duration(seconds: 1));
        set(c, 'chan-a');

        // The session rebuilds for the second community before the first
        // community's manager is retired. Until then that manager must neither
        // flush its pending change through it nor take in what it delivers.
        config.switchTo('https://second.example');
        c.read(relaySessionProvider);
        final (headTag, payload) = secondHead('chan-b');
        final head = relay.event(
          headTag,
          payload,
          nowSeconds() + 1,
          tags: [
            if (dTag == 'read-state') ['t', 'read-state'],
          ],
        );
        relay.stored.add(head);
        if (history == null) {
          relay.emit(head);
        } else {
          history.complete();
        }
        clock.elapse(const Duration(seconds: 10));
        final sentToSecond = [
          for (var i = 0; i < relay.published.length; i++)
            if (relay.publishedTo[i] == 'https://second.example')
              jsonEncode(relay.decrypt(relay.published[i])),
        ];
        expect(sentToSecond, everyElement(isNot(contains('chan-a'))));
        expect(values(c), {'chan-b'});
        if (dTag == 'read-state') {
          expect(relay.snapshotHosts, contains('second.example'));
        }

        config.switchTo('https://relay.example');
        clock.elapse(const Duration(seconds: 1));
        expect(values(c), {'chan-a'});
        c.dispose();
      });
    }
  }

  for (final lane in _lanes) {
    Map<String, Object> blob(Map<String, (bool, int)> entries) => {
      'version': 1,
      'channels': {
        for (final e in entries.entries)
          e.key: {lane.field: e.value.$1, 'updatedAt': e.value.$2},
      },
    };

    group(lane.dTag, () {
      late SidebarRelay relay;
      late ProviderContainer container;
      late _Subject subject;
      late int t;
      setUp(() {
        relay = SidebarRelay();
        container = ProviderContainer(
          overrides: [
            relayConfigProvider.overrideWith(() => _Config(relay.keys.nsec)),
            relaySessionProvider.overrideWith(() => relay.session),
          ],
        );
        container.read(relaySessionProvider);
        t = nowSeconds();
      });
      tearDown(() {
        subject.dispose();
        container.dispose();
      });

      _Subject start(FakeAsync clock, {SharedPreferences? store}) {
        subject = lane.build(store ?? prefs, relay);
        subject.init();
        clock.flushMicrotasks();
        return subject;
      }

      for (final priorHead in [false, true]) {
        fakeAsyncTest('OK before echo keeps a coherent cursor '
            '(${priorHead ? 'stale prior id' : 'no prior id'})', (clock) {
          const low =
              '0000000000000000000000000000000000000000000000000000000000000000';
          if (priorHead) {
            relay.stored.add(relay.event(lane.dTag, blob({}), t + 30));
          }
          start(clock).set('mine', true);
          clock.elapse(const Duration(seconds: 5));
          final own = relay.published.single;
          expect(
            own.createdAt,
            priorHead ? t + 31 : inInclusiveRange(t, nowSeconds()),
          );

          // The OK beat the (never delivered) echo. A same-second peer that
          // the relay retains over our event must still be adopted.
          // A same-second peer with a higher ID must still lose.
          final peerId = '${low.substring(1)}1';
          expect(peerId.compareTo(own.id), lessThan(0));
          for (final (name, id) in [
            ('loser', ''.padLeft(64, 'f')),
            ('peer', peerId),
          ]) {
            relay.emit(
              relay.event(
                lane.dTag,
                blob({name: (true, own.createdAt)}),
                own.createdAt,
                id: id,
              ),
            );
          }
          clock.elapse(const Duration(milliseconds: 20));
          expect(subject.values(), {'peer': true});
        });
      }

      for (final afterFallback in [false, true]) {
        fakeAsyncTest(
          'terminal CLOSED ${afterFallback ? 'after' : 'before'} the readiness '
          'fallback is not re-sent; history still recovers',
          (clock) {
            relay
              ..historyFailures = 1
              ..stored.add(relay.event(lane.dTag, blob({'a': (true, t)}), t));
            if (afterFallback) {
              relay.withholdEose = true;
            } else {
              relay.rejectLive = 'restricted: not allowed';
            }
            start(clock);
            clock.elapse(const Duration(milliseconds: 600));
            if (afterFallback) {
              relay.closeLive(lane.dTag, 'error: too many subscriptions');
            }
            clock.elapse(const Duration(minutes: 5));
            expect(relay.reqsFor(lane.dTag, 'l-'), hasLength(1));
            expect(relay.reqsFor(lane.dTag, 'h-'), hasLength(2));
            expect(subject.values(), {'a': true});
          },
        );
      }

      for (final error in [null, StateError('disk full')]) {
        fakeAsyncTest(
          'failed persist (${error == null ? 'false' : 'throws'}) retries the '
          'same head until it is durable',
          (clock) {
            final flaky = FlakyPrefs(prefs)
              ..failures = 1
              ..error = error;
            relay.stored.add(relay.event(lane.dTag, blob({'a': (true, t)}), t));
            start(clock, store: flaky);
            expect(prefs.getKeys(), isEmpty);
            clock.elapse(const Duration(seconds: 2));
            expect(relay.reqsFor(lane.dTag, 'h-'), hasLength(2));
            expect(prefs.getKeys(), hasLength(1));
            expect(subject.values(), {'a': true});
          },
        );
      }

      fakeAsyncTest('an undecodable head does not hold startup open', (clock) {
        relay.stored.add(relay.event(lane.dTag, 'garbage', t));
        start(clock);
        clock.elapse(const Duration(seconds: 70));
        expect(relay.reqsFor(lane.dTag, 'h-'), hasLength(1));
        expect(relay.reqsFor(lane.dTag, 'l-'), hasLength(1));
      });

      fakeAsyncTest('merges legacy seconds with modern milliseconds', (clock) {
        final millis = (t - 110) * 1000;
        relay.stored.add(
          relay.event(
            lane.dTag,
            blob({'mixed': (true, millis), 'toggled': (true, (t + 90) * 1000)}),
            t,
          ),
        );
        start(clock);
        expect(subject.values(), {'mixed': true, 'toggled': true});

        relay.stored.add(
          relay.event(
            lane.dTag,
            blob({'mixed': (false, t - 100), 'other': (true, t - 90)}),
            t + 1,
          ),
        );
        subject.set('toggled', false);
        clock.elapse(const Duration(seconds: 5));

        final payload = relay.decrypt(relay.published.single) as Map;
        expect(payload['channels'], {
          'mixed': {lane.field: false, 'updatedAt': t - 100},
          'other': {lane.field: true, 'updatedAt': t - 90},
          'toggled': {lane.field: false, 'updatedAt': (t + 90) * 1000 + 1},
        });
        expect(subject.values(), {
          'mixed': false,
          'toggled': false,
          'other': true,
        });
      });

      fakeAsyncTest('repairs oversized heads and evicts the oldest clears', (
        clock,
      ) {
        final active = {
          for (var i = 0; i < 498; i++) 'active-$i': (true, t - 70),
        };
        relay.stored.add(
          relay.event(
            lane.dTag,
            blob({
              ...active,
              'older-clear': (false, (t - 100) * 1000),
              'newer-clear': (false, t - 90),
              'newest-clear': (false, t - 80),
            }),
            t,
          ),
        );
        start(clock).set('active-0', false);
        clock.elapse(const Duration(seconds: 5));

        final payload = relay.decrypt(relay.published.single) as Map;
        final channels = payload['channels'] as Map;
        expect(channels, hasLength(500));
        expect(channels, isNot(contains('older-clear')));
        expect(channels['newer-clear']['updatedAt'], t - 90);
        expect(channels['active-0'][lane.field], isFalse);
        expect(channels['active-1'][lane.field], isTrue);
        expect(subject.values(), hasLength(500));
      });

      for (final head in <String, Object?>{
        'failed fetch': null,
        'bad payload': 'garbage',
        'unknown version': {'version': 2, 'channels': {}},
        'bad entry': {
          'version': 1,
          'channels': {
            'bad': {lane.field: true},
          },
        },
      }.entries) {
        fakeAsyncTest('does not publish after ${head.key}', (clock) {
          start(clock);
          if (head.value == null) {
            relay.writerFailures = 1;
          } else {
            relay.stored.add(relay.event(lane.dTag, head.value!, t));
          }
          subject.set('kept-local', true);
          clock.elapse(const Duration(seconds: 5));
          expect(relay.published, isEmpty);
          expect(subject.values(), {'kept-local': true});

          relay.stored.clear();
          if (head.value == null) {
            clock.elapse(const Duration(seconds: 4));
          } else {
            final valid = relay.event(lane.dTag, blob({}), t + 1);
            relay.stored.add(valid);
            relay.emit(valid);
            clock.elapse(const Duration(seconds: 6));
          }
          final payload = relay.decrypt(relay.published.single) as Map;
          expect((payload['channels'] as Map).keys, ['kept-local']);
        });
      }

      String legacyCacheKey() => lane.dTag == 'channel-stars'
          ? channelStarsKey(relay.pubkey, 'https://relay.example')
          : channelMutesKey(relay.pubkey, 'https://relay.example');

      fakeAsyncTest('upgrading recovers offline flags below the cap', (clock) {
        final local = blob({
          'offline-clear': (false, t + 1),
          'offline-add': (true, t + 2),
          'older-absent': (true, t - 10),
          'absent-clear': (false, t + 3),
          'equal-clear': (false, t),
          'same-second-absent': (true, t),
          'changed-by-peer': (true, t - 2),
          'already-current': (true, t - 1),
        });
        unawaited(prefs.setString(legacyCacheKey(), jsonEncode(local)));
        relay.stored.add(
          relay.event(
            lane.dTag,
            blob({
              'offline-clear': (true, (t - 1) * 1000),
              'equal-clear': (true, t * 1000),
              'changed-by-peer': (false, t * 1000),
              'already-current': (true, t - 2),
            }),
            t,
          ),
        );

        start(clock);
        expect(subject.values(), {
          'offline-clear': false,
          'offline-add': true,
          'older-absent': true,
          'same-second-absent': true,
          'equal-clear': false,
          'changed-by-peer': false,
          'already-current': true,
        });
        final migrated = jsonDecode(prefs.getString(legacyCacheKey())!) as Map;
        expect(
          migrated['pending'].keys,
          unorderedEquals([
            'offline-clear',
            'offline-add',
            'older-absent',
            'same-second-absent',
            'equal-clear',
          ]),
        );
        expect(migrated.containsKey('legacyCache'), isFalse);

        clock.elapse(const Duration(seconds: 5));
        final payload = relay.decrypt(relay.published.single) as Map;
        expect(payload['channels'], {
          'offline-clear': {lane.field: false, 'updatedAt': (t + 1) * 1000},
          'offline-add': {lane.field: true, 'updatedAt': (t + 2) * 1000},
          'older-absent': {lane.field: true, 'updatedAt': (t - 10) * 1000},
          'same-second-absent': {lane.field: true, 'updatedAt': t * 1000},
          'equal-clear': {lane.field: false, 'updatedAt': t * 1000 + 1},
          'changed-by-peer': {lane.field: false, 'updatedAt': t * 1000},
          'already-current': {lane.field: true, 'updatedAt': t - 2},
        });
      });

      fakeAsyncTest('upgrading at the cap avoids restoring evicted flags', (
        clock,
      ) {
        final cached = blob({
          'evicted': (true, t - 10),
          'same-second-absent': (true, t),
          'offline-add': (true, t + 2),
          'absent-clear': (false, t + 3),
        });
        unawaited(prefs.setString(legacyCacheKey(), jsonEncode(cached)));
        relay.stored.add(
          relay.event(
            lane.dTag,
            blob({for (var i = 0; i < 500; i++) 'active-$i': (true, t - 2)}),
            t,
          ),
        );

        start(clock);
        final cache = jsonDecode(prefs.getString(legacyCacheKey())!) as Map;
        expect(cache['pending'].keys, ['offline-add']);
        clock.elapse(const Duration(seconds: 5));
        final payload = relay.decrypt(relay.published.single) as Map;
        final channels = payload['channels'] as Map;
        expect(channels, hasLength(500));
        expect(channels['offline-add'], {
          lane.field: true,
          'updatedAt': (t + 2) * 1000,
        });
        expect(channels, isNot(contains('evicted')));
        expect(channels, isNot(contains('same-second-absent')));
        expect(channels, isNot(contains('absent-clear')));
      });

      fakeAsyncTest('upgrading with no writer head recovers absent flags', (
        clock,
      ) {
        final cached = blob({'offline-add': (true, t - 10)});
        unawaited(prefs.setString(legacyCacheKey(), jsonEncode(cached)));
        start(clock);
        clock.elapse(const Duration(seconds: 5));
        final payload = relay.decrypt(relay.published.single) as Map;
        expect(payload['channels'], {
          'offline-add': {lane.field: true, 'updatedAt': (t - 10) * 1000},
        });
      });

      Map readFlagCache() =>
          jsonDecode(prefs.getString(legacyCacheKey())!) as Map;

      void seedAbsentClear() {
        unawaited(
          prefs.setString(
            legacyCacheKey(),
            jsonEncode(blob({'x': (false, t - 1)})),
          ),
        );
        relay.stored.add(relay.event(lane.dTag, blob({'y': (true, t)}), t));
      }

      NostrEvent returningFlag(bool enabled, int updatedAt) => relay.event(
        lane.dTag,
        blob({'y': (true, t), 'x': (enabled, updatedAt)}),
        t + 1,
      );

      for (final scenario in [
        'live',
        'rebuild',
        'equal stamp',
        'writer read',
      ]) {
        fakeAsyncTest('a kept clear wins on $scenario', (clock) {
          seedAbsentClear();
          start(clock);
          clock.elapse(const Duration(seconds: 6));
          expect(relay.published, isEmpty);
          expect(subject.values(), {'y': true});
          expect(readFlagCache()['keptClears'], {
            'x': {lane.field: false, 'updatedAt': t - 1},
          });
          expect(readFlagCache()['pending'], isEmpty);

          if (scenario == 'rebuild') {
            subject.dispose();
            start(clock);
          }
          final clearMillis = (t - 1) * 1000;
          final equal = scenario == 'equal stamp';
          final returning = returningFlag(true, equal ? clearMillis : (t - 2));
          relay.stored.add(returning);
          if (scenario == 'writer read') {
            relay.holdOk = Completer<void>();
            subject.set('unrelated', true);
            clock.elapse(const Duration(seconds: 5));
            expect(subject.values()['x'], isFalse);
            final ok = relay.holdOk!;
            relay.holdOk = null;
            ok.complete();
            clock.flushMicrotasks();
          } else {
            relay.emit(returning);
            clock.elapse(const Duration(milliseconds: 20));
            expect(subject.values()['x'], isFalse);
            expect(readFlagCache()['pending']['x'], {
              lane.field: false,
              'updatedAt': clearMillis,
            });
            clock.elapse(const Duration(seconds: 5));
          }
          final payload = relay.decrypt(relay.published.single) as Map;
          expect(payload['channels']['x'], {
            lane.field: false,
            'updatedAt': clearMillis + (equal ? 1 : 0),
          });
          if (scenario == 'writer read') {
            expect(payload['channels']['unrelated'][lane.field], isTrue);
          }
          expect(readFlagCache().containsKey('keptClears'), isFalse);
          expect(readFlagCache()['pending'], isEmpty);
        });
      }

      for (final enabled in [true, false]) {
        fakeAsyncTest(
          'a kept clear is retired by a ${enabled ? 'newer enable' : 'confirmed clear'}',
          (clock) {
            seedAbsentClear();
            start(clock);
            final returning = returningFlag(enabled, enabled ? t : t - 2);
            relay.stored.add(returning);
            relay.emit(returning);
            clock.elapse(const Duration(seconds: 6));
            expect(subject.values()['x'], enabled);
            expect(readFlagCache().containsKey('keptClears'), isFalse);
            expect(relay.published, isEmpty);
          },
        );
      }

      fakeAsyncTest('a tap replaces a kept clear', (clock) {
        seedAbsentClear();
        start(clock).set('x', true);
        expect(readFlagCache().containsKey('keptClears'), isFalse);
        clock.elapse(const Duration(seconds: 5));
        final payload = relay.decrypt(relay.published.single) as Map;
        expect(payload['channels']['x'][lane.field], isTrue);
      });

      fakeAsyncTest('a kept clear is promoted when readback finds a peer', (
        clock,
      ) {
        seedAbsentClear();
        start(clock).set('unrelated', true);
        relay.replaceNextPublish = (attempt) => relay.event(
          lane.dTag,
          blob({'y': (true, t), 'x': (true, t - 2)}),
          attempt.createdAt + 1,
        );
        clock.elapse(const Duration(seconds: 5));
        expect(relay.published, hasLength(1));
        expect(subject.values()['x'], isFalse);
        expect(readFlagCache().containsKey('keptClears'), isFalse);
        expect(readFlagCache()['pending']['x'][lane.field], isFalse);

        clock.elapse(const Duration(seconds: 4));
        expect(relay.published, hasLength(2));
        final payload = relay.decrypt(relay.published.last) as Map;
        expect(payload['channels']['x'][lane.field], isFalse);
        expect(payload['channels']['unrelated'][lane.field], isTrue);
      });

      fakeAsyncTest('absent clears use no slots while migration reads', (
        clock,
      ) {
        final cached = blob({
          for (var i = 0; i < 500; i++) 'clear-$i': (false, t - 2),
          'active': (true, t - 1),
        });
        unawaited(prefs.setString(legacyCacheKey(), jsonEncode(cached)));
        final held = relay.holdWriter = Completer<void>();
        start(clock).set('new', true);
        expect(relay.published, isEmpty);
        relay.holdWriter = null;
        held.complete();
        clock.flushMicrotasks();
        clock.elapse(const Duration(seconds: 5));
        final payload = relay.decrypt(relay.published.single) as Map;
        expect(payload['channels'].keys, unorderedEquals(['active', 'new']));
        expect(readFlagCache()['keptClears'], hasLength(500));
      });

      fakeAsyncTest('a kept clear replaces its entry at the cap', (clock) {
        seedAbsentClear();
        start(clock);
        final full = {
          for (var i = 0; i < 499; i++) 'active-$i': (true, t),
          'x': (true, t - 2),
        };
        final returning = relay.event(lane.dTag, blob(full), t + 1);
        relay.stored.add(returning);
        relay.emit(returning);
        clock.elapse(const Duration(seconds: 6));
        final payload = relay.decrypt(relay.published.single) as Map;
        expect(payload['channels'].keys, unorderedEquals(full.keys));
        expect(payload['channels']['x'][lane.field], isFalse);
      });

      fakeAsyncTest('an oversized migration journal drains across a rebuild', (
        clock,
      ) {
        final cached = blob({
          for (var i = 0; i < 501; i++) 'active-$i': (true, t - 1000 + i),
        });
        unawaited(prefs.setString(legacyCacheKey(), jsonEncode(cached)));
        start(clock);
        expect(readFlagCache()['pending'], hasLength(501));
        clock.elapse(const Duration(seconds: 5));
        final first = relay.decrypt(relay.published.single) as Map;
        expect(first['channels'], hasLength(500));
        expect(first['channels'], isNot(contains('active-500')));
        expect(readFlagCache()['pending'].keys, ['active-500']);

        subject.dispose();
        start(clock);
        clock.elapse(const Duration(seconds: 5));
        expect(relay.published, hasLength(2));
        final last = relay.decrypt(relay.published.last) as Map;
        expect(last['channels'], hasLength(500));
        expect(last['channels']['active-500'][lane.field], isTrue);
        expect(last['channels'], isNot(contains('active-0')));
        expect(readFlagCache()['pending'], isEmpty);
      });

      fakeAsyncTest('clearing an absent flag follows the lane policy', (clock) {
        start(clock).set('absent', false);
        clock.elapse(const Duration(seconds: 5));
        if (lane.dTag == 'channel-stars') {
          expect(subject.values(), isEmpty);
          expect(relay.published, isEmpty);
        } else {
          final payload = relay.decrypt(relay.published.single) as Map;
          expect(payload['channels']['absent'][lane.field], isFalse);
        }
      });

      fakeAsyncTest('an offline toggle keeps legacy migration retryable', (
        clock,
      ) {
        final cached = blob({'offline-clear': (false, t - 2)});
        unawaited(prefs.setString(legacyCacheKey(), jsonEncode(cached)));
        relay.stored.add(
          relay.event(lane.dTag, blob({'offline-clear': (true, t - 5)}), t - 4),
        );
        relay.writerFailures = 1;
        start(clock).set('new-edit', true);
        expect(relay.published, isEmpty);
        final offline = jsonDecode(prefs.getString(legacyCacheKey())!) as Map;
        expect(offline['legacyCache'], isTrue);
        subject.dispose();

        start(clock);
        clock.elapse(const Duration(seconds: 5));
        final payload = relay.decrypt(relay.published.single) as Map;
        expect(payload['channels']['offline-clear'][lane.field], isFalse);
        expect(payload['channels']['new-edit'][lane.field], isTrue);
      });

      fakeAsyncTest('legacy migration waits for a readable writer head', (
        clock,
      ) {
        final cached = blob({'offline-clear': (false, t - 2)});
        unawaited(prefs.setString(legacyCacheKey(), jsonEncode(cached)));
        relay.stored.add(relay.event(lane.dTag, 'garbage', t - 5));
        start(clock);

        clock.elapse(const Duration(minutes: 2));
        expect(relay.writerQueries, [lane.dTag]);
        expect(relay.published, isEmpty);
        expect(prefs.getString(legacyCacheKey()), jsonEncode(cached));

        final valid = relay.event(
          lane.dTag,
          blob({'offline-clear': (true, t - 4)}),
          t - 4,
        );
        relay.stored.add(valid);
        relay.emit(valid);
        clock.elapse(const Duration(seconds: 6));
        final payload = relay.decrypt(relay.published.single) as Map;
        expect(payload['channels']['offline-clear'][lane.field], isFalse);
      });

      fakeAsyncTest(
        'a stale writer read retries without replacing newer flags',
        (clock) {
          final old = relay.event(lane.dTag, blob({'old': (true, t - 2)}), t);
          final recent = relay.event(
            lane.dTag,
            blob({'recent': (true, t)}),
            t + 1,
          );
          relay.stored.add(recent);
          start(clock).set('pending', true);
          relay.stored
            ..clear()
            ..add(old);
          clock.elapse(const Duration(seconds: 5));
          expect(relay.published, isEmpty);
          expect(subject.values(), {'recent': true, 'pending': true});

          relay.stored.add(recent);
          clock.elapse(const Duration(seconds: 4));
          final payload = relay.decrypt(relay.published.single) as Map;
          expect(
            payload['channels'].keys,
            unorderedEquals(['recent', 'pending']),
          );
        },
      );

      fakeAsyncTest('an unrelated publish preserves a legacy writer toggle', (
        clock,
      ) {
        relay.stored.add(
          relay.event(lane.dTag, blob({'legacy': (true, t - 2)}), t),
        );
        start(clock).set('unrelated', true);
        clock.elapse(const Duration(seconds: 5));
        final sent = relay.published.single;
        final payload = relay.decrypt(sent) as Map;
        final flags = payload['channels'] as Map;
        expect(flags['legacy']['updatedAt'], t - 2);
        // An old client compares raw timestamps.
        expect(t > flags['legacy']['updatedAt'], isTrue);
        final old = relay.event(lane.dTag, {
          ...payload,
          'channels': {
            ...flags,
            'legacy': {lane.field: false, 'updatedAt': t},
          },
        }, sent.createdAt + 1);
        relay.stored.add(old);
        subject.refresh();
        clock.flushMicrotasks();
        expect(subject.values(), {'legacy': false, 'unrelated': true});
      });

      fakeAsyncTest('a returning phone cannot resurrect an evicted clear', (
        clock,
      ) {
        relay.stored.add(
          relay.event(
            lane.dTag,
            blob({for (var i = 0; i < 500; i++) 'flag-$i': (true, t - 2)}),
            t,
          ),
        );
        final returningPhone = start(clock);
        final peer = lane.build(SidebarDevicePrefs(), relay);
        peer.init();
        clock.flushMicrotasks();
        peer.set('flag-0', false);
        clock.elapse(const Duration(seconds: 5));
        peer.set('new', true);
        clock.elapse(const Duration(seconds: 5));
        expect(peer.values()['flag-0'], isFalse);
        expect(returningPhone.values()['flag-0'], isTrue);

        // Once the clear ages out of the latest retained head, the returning
        // device still has its old star. It must only send its current edit.
        final sent = relay.published.last;
        final retained = relay.decrypt(sent) as Map;
        final flags = Map.of(retained['channels'] as Map)..remove('flag-0');
        relay.stored.add(
          relay.event(lane.dTag, {
            'version': 1,
            'channels': {
              ...flags,
              // A later writer can have an earlier clock. A stale-cache
              // union would evict this flag and leave flag-0 in the record.
              'another': {lane.field: true, 'updatedAt': t - 100},
            },
          }, sent.createdAt + 1),
        );
        returningPhone.set('flag-2', false);
        clock.elapse(const Duration(seconds: 5));
        final payload = relay.decrypt(relay.published.last) as Map;
        expect(payload['channels'], isNot(contains('flag-0')));
        expect(payload['channels']['flag-2'][lane.field], isFalse);
        expect(payload['channels']['another'][lane.field], isTrue);
        expect(returningPhone.values(), isNot(contains('flag-0')));
        peer.dispose();
      });

      fakeAsyncTest('a pending clear survives manager replacement at the cap', (
        clock,
      ) {
        relay.stored.add(
          relay.event(
            lane.dTag,
            blob({
              'pending': (true, t - 10),
              for (var i = 0; i < 499; i++) 'active-$i': (true, t + 60),
            }),
            t,
          ),
        );
        start(clock).set('pending', false);
        relay.writerFailures = 1;
        clock.elapse(const Duration(seconds: 5));
        expect(relay.published, isEmpty);
        subject.dispose();
        start(clock);
        expect(subject.values()['pending'], isFalse);
        subject.set('new', true);
        clock.elapse(const Duration(seconds: 5));
        final payload = relay.decrypt(relay.published.single) as Map;
        final channels = payload['channels'] as Map;
        expect(channels, hasLength(500));
        expect(channels['pending'][lane.field], isFalse);
        expect(channels['new'][lane.field], isTrue);
      });

      fakeAsyncTest('a duplicate OK retries from the actual winning head', (
        clock,
      ) {
        start(clock).set('pending', true);
        relay.replaceNextPublish = (sent) => relay.event(
          lane.dTag,
          blob({'peer': (true, t - 1)}),
          sent.createdAt + 1,
        );
        clock.elapse(const Duration(seconds: 5));
        expect(relay.published, hasLength(1));
        expect(subject.values(), {'pending': true, 'peer': true});
        clock.elapse(const Duration(seconds: 4));
        expect(relay.published, hasLength(2));
        expect(relay.writerQueries, List.filled(4, lane.dTag));
        final payload = relay.decrypt(relay.published.last) as Map;
        expect(payload['channels'].keys, unorderedEquals(['peer', 'pending']));
        final attempted = relay.decrypt(relay.published.first) as Map;
        expect(
          payload['channels']['pending'],
          attempted['channels']['pending'],
        );
      });

      fakeAsyncTest('a newer writer flag supersedes an offline edit', (clock) {
        start(clock).set('offline', true);
        relay.writerFailures = 1;
        clock.elapse(const Duration(seconds: 5));
        expect(relay.published, isEmpty);
        subject.dispose();

        // The peer wrote in seconds after the phone's millisecond edit.
        final newer = relay.event(
          lane.dTag,
          blob({'offline': (false, t + 60)}),
          t + 1,
        );
        relay.stored.add(newer);
        start(clock);
        clock.elapse(const Duration(seconds: 5));
        expect(relay.published, isEmpty);
        expect(subject.values(), {'offline': false});
        final cache = jsonDecode(prefs.getString(legacyCacheKey())!) as Map;
        expect(cache['pending'], isEmpty);
      });

      fakeAsyncTest('a fresh tap advances past a newer observed peer', (clock) {
        relay.stored.add(
          relay.event(lane.dTag, blob({'flag': (true, t - 2)}), t),
        );
        start(clock).set('flag', false);
        final peerTime = (t + 60) * 1000;
        final peer = relay.event(
          lane.dTag,
          blob({'flag': (false, peerTime)}),
          t + 1,
        );
        relay.stored.add(peer);
        relay.emit(peer);
        clock.elapse(const Duration(milliseconds: 20));

        subject.set('flag', true);
        clock.elapse(const Duration(seconds: 5));
        final payload = relay.decrypt(relay.published.single) as Map;
        expect(payload['channels']['flag'], {
          lane.field: true,
          'updatedAt': peerTime + 1,
        });
      });

      fakeAsyncTest('a mixed batch sends only edits that still win', (clock) {
        start(clock).set('offline', true);
        subject.set('untouched-by-peer', true);
        final cache = jsonDecode(prefs.getString(legacyCacheKey())!) as Map;
        final originalEdit = cache['pending']['untouched-by-peer'];
        final remoteTime = (t + 60) * 1000;
        relay.stored.add(
          relay.event(lane.dTag, blob({'offline': (false, remoteTime)}), t + 1),
        );
        clock.elapse(const Duration(seconds: 5));
        final payload = relay.decrypt(relay.published.single) as Map;
        expect(payload['channels'], {
          'offline': {lane.field: false, 'updatedAt': remoteTime},
          'untouched-by-peer': originalEdit,
        });
        final saved = jsonDecode(prefs.getString(legacyCacheKey())!) as Map;
        expect(saved['pending'], isEmpty);
      });

      for (final peerEdited in [false, true]) {
        fakeAsyncTest(
          'losing readback retires a flag ${peerEdited ? 'superseded by a peer' : 'already retained'}',
          (clock) {
            final ok = relay.holdOk = Completer<void>();
            start(clock).set('clicked', true);
            clock.elapse(const Duration(seconds: 5));
            final sent = relay.published.single;
            relay.holdOk = null;
            relay.writerFailures = 1;
            ok.complete();
            clock.flushMicrotasks();

            if (peerEdited) {
              relay.stored.add(
                relay.event(
                  lane.dTag,
                  blob({'clicked': (false, (t + 60) * 1000)}),
                  sent.createdAt + 1,
                ),
              );
            }
            clock.elapse(const Duration(seconds: 4));
            expect(relay.published, [sent]);
            expect(subject.values(), {'clicked': !peerEdited});
            final cache = jsonDecode(prefs.getString(legacyCacheKey())!) as Map;
            expect(cache['pending'], isEmpty);
          },
        );
      }

      fakeAsyncTest('a confirmed clear loses eviction priority at the cap', (
        clock,
      ) {
        relay.stored.add(
          relay.event(
            lane.dTag,
            blob({
              'cleared': (true, t - 10),
              for (var i = 0; i < 499; i++) 'active-$i': (true, t + 300),
            }),
            t,
          ),
        );
        final ok = relay.holdOk = Completer<void>();
        start(clock).set('cleared', false);
        clock.elapse(const Duration(seconds: 5));
        relay.emit(relay.published.single);
        clock.elapse(const Duration(milliseconds: 20));
        subject.set('cleared', false);
        subject.set('new', true);

        relay.holdOk = null;
        ok.complete();
        clock.flushMicrotasks();
        clock.elapse(const Duration(seconds: 2));
        final payload = relay.decrypt(relay.published.last) as Map;
        expect(relay.published, hasLength(2));
        expect(payload['channels'], hasLength(500));
        expect(payload['channels'], isNot(contains('cleared')));
        expect(payload['channels']['active-0'][lane.field], isTrue);
        expect(payload['channels']['new'][lane.field], isTrue);
      });

      fakeAsyncTest('confirmation preserves a newer edit to the same flag', (
        clock,
      ) {
        relay.holdOk = Completer<void>();
        start(clock).set('flag', true);
        clock.elapse(const Duration(seconds: 5));
        expect(relay.published, hasLength(1));
        subject.set('flag', false);
        final ok = relay.holdOk!;
        relay.holdOk = null;
        ok.complete();
        clock.flushMicrotasks();
        expect(subject.values(), {'flag': false});
        clock.elapse(const Duration(seconds: 2));
        final payload = relay.decrypt(relay.published.last) as Map;
        expect(relay.published, hasLength(2));
        expect(payload['channels']['flag'][lane.field], isFalse);
      });

      fakeAsyncTest('resume adopts a head the healthy socket missed', (clock) {
        relay.stored.add(relay.event(lane.dTag, blob({'a': (true, t)}), t));
        start(clock);
        expect(subject.values(), {'a': true});
        relay.stored.add(
          relay.event(
            lane.dTag,
            blob({'a': (true, t), 'b': (true, t + 1)}),
            t + 1,
          ),
        );
        subject.refresh();
        clock.flushMicrotasks();
        expect(subject.values(), {'a': true, 'b': true});
        expect(prefs.getString(prefs.getKeys().single), contains('"b"'));
        expect(relay.reqsFor(lane.dTag, 'l-'), hasLength(1));

        // A failed resume read is one shot, not a retry loop.
        relay.historyFailures = 1;
        subject.refresh();
        clock.elapse(const Duration(seconds: 70));
        expect(relay.reqsFor(lane.dTag, 'h-'), hasLength(3));
      });

      fakeAsyncTest(
        'a read held across a local edit defers instead of overwriting it',
        (clock) {
          start(clock);
          // The peer's clock runs one second ahead of ours on the same channel.
          relay.stored.add(
            relay.event(lane.dTag, blob({'c': (false, t + 1)}), t + 1),
          );
          final held = relay.holdHistory = Completer<void>();
          subject.refresh();
          clock.flushMicrotasks();
          subject.set('c', true);
          held.complete();
          relay.holdHistory = null;
          clock.flushMicrotasks();
          expect(subject.values(), {'c': true});

          // Resume while the edit is still pending leaves it alone too.
          subject.refresh();
          clock.flushMicrotasks();
          expect(subject.values(), {'c': true});
          expect(relay.reqsFor(lane.dTag, 'h-'), hasLength(2));
        },
      );
    });
  }
}

/// Whole-blob lanes: resume re-read, and sections' publication cursor.
void wholeBlobLanes(SharedPreferences Function() prefs) {
  group('sections', () {
    late SidebarRelay relay;
    late ChannelSectionsManager m;
    setUp(() => relay = SidebarRelay());
    tearDown(() => m.dispose(flushPending: false));

    Map<String, Object> blob(String name) => {
      'version': 1,
      'sections': [
        {'id': name, 'name': name, 'order': 0},
      ],
      'assignments': <String, String>{},
    };

    ChannelSectionsManager start(FakeAsync clock) {
      m = ChannelSectionsManager(
        pubkey: relay.pubkey,
        relayUrl: 'https://relay.example',
        prefs: prefs(),
        crypto: ChannelSectionsCrypto(relay.keys.nsec, relay.pubkey),
        relaySession: relay.session,
        signedEventRelay: relay.signer,
        remoteEnabled: true,
        onChanged: () {},
      )..initialize();
      clock.flushMicrotasks();
      return m;
    }

    List<String> names() => [for (final s in m.store.sections) s.name];

    fakeAsyncTest('resume adopts a head the healthy socket missed', (clock) {
      final t = nowSeconds();
      start(clock);
      relay.stored.add(relay.event('channel-sections', blob('later'), t));
      m.refreshFromRelay();
      clock.flushMicrotasks();
      expect(names(), ['later']);
      expect(relay.reqsFor('channel-sections', 'l-'), hasLength(1));
    });

    fakeAsyncTest('resume leaves a pending edit alone', (clock) {
      final t = nowSeconds();
      start(clock).createSection('mine');
      relay.stored.add(relay.event('channel-sections', blob('peer'), t + 9));
      m.refreshFromRelay();
      clock.flushMicrotasks();
      expect(names(), ['mine']);
    });

    fakeAsyncTest('OK before echo keeps a coherent cursor', (clock) {
      start(clock).createSection('mine');
      clock.elapse(const Duration(seconds: 5));
      final own = relay.published.single;
      // A same-second higher ID loses to our own event; a lower one wins.
      for (final (name, pad, want) in [
        ('loser', 'f', 'mine'),
        ('peer', '0', 'peer'),
      ]) {
        relay.emit(
          relay.event(
            'channel-sections',
            blob(name),
            own.createdAt,
            id: ''.padLeft(64, pad),
          ),
        );
        clock.elapse(const Duration(milliseconds: 20));
        expect(names(), [want]);
      }
    });

    fakeAsyncTest('a same-second loser merged during the OK wait is undone', (
      clock,
    ) {
      relay.holdOk = Completer<void>();
      start(clock).createSection('mine');
      clock.elapse(const Duration(seconds: 5));
      final own = relay.published.single;
      final loser = relay.event(
        'channel-sections',
        blob('loser'),
        own.createdAt,
        id: ''.padLeft(64, 'f'),
      );
      relay
        ..stored.add(loser)
        ..emit(loser);
      clock.elapse(const Duration(milliseconds: 20));
      expect(names(), ['loser']);
      relay.holdOk!.complete();
      clock.flushMicrotasks();
      relay.emit(own);
      m.refreshFromRelay();
      clock.elapse(const Duration(milliseconds: 20));
      expect(names(), ['mine']);
      expect(prefs().getString(prefs().getKeys().single), contains('"mine"'));
    });

    fakeAsyncTest('a same-second loser adopted after an edit re-converges', (
      clock,
    ) {
      relay.holdOk = Completer<void>();
      start(clock).createSection('mine');
      clock.elapse(const Duration(seconds: 5));
      final own = relay.published.single;
      m.renameSection(m.store.sections.single.id, 'new-local');
      final loser = relay.event(
        'channel-sections',
        blob('loser'),
        own.createdAt,
        id: ''.padLeft(64, 'f'),
      );
      relay
        ..stored.add(loser)
        ..emit(loser);
      clock.elapse(const Duration(milliseconds: 20));
      expect(names(), ['loser']);
      relay.holdOk!.complete();
      relay.holdOk = null;
      clock.flushMicrotasks();
      clock.elapse(const Duration(seconds: 6));
      final heads = relay.stored.toList()
        ..sort(
          (a, b) => a.createdAt != b.createdAt
              ? b.createdAt.compareTo(a.createdAt)
              : a.id.compareTo(b.id),
        );
      final retained = relay.decrypt(heads.first) as Map<String, dynamic>;
      relay.emit(heads.first);
      m.refreshFromRelay();
      clock.elapse(const Duration(milliseconds: 20));
      expect(names(), [
        for (final s in retained['sections'] as List) s['name'],
      ]);
      expect(prefs().getString(prefs().getKeys().single), jsonEncode(retained));
    });

    fakeAsyncTest(
      'a retired flush pre-read cannot rewrite the successor cache',
      (clock) {
        start(clock).createSection('mine');
        final held = relay.holdHistory = Completer<void>();
        m.dispose();
        clock.flushMicrotasks();
        m = ChannelSectionsManager(
          pubkey: relay.pubkey,
          relayUrl: 'https://relay.example',
          prefs: prefs(),
          crypto: ChannelSectionsCrypto(relay.keys.nsec, relay.pubkey),
          relaySession: relay.session,
          signedEventRelay: relay.signer,
          remoteEnabled: false,
          onChanged: () {},
        )..initialize();
        m.renameSection(m.store.sections.single.id, 'successor-local');
        clock.flushMicrotasks();
        final before = prefs().getString(prefs().getKeys().single);
        held.complete();
        relay.holdHistory = null;
        clock.flushMicrotasks();
        expect(names(), ['successor-local']);
        expect(prefs().getString(prefs().getKeys().single), before);
      },
    );

    fakeAsyncTest('a local edit during the OK wait still publishes', (clock) {
      relay.holdOk = Completer<void>();
      start(clock).createSection('mine');
      clock.elapse(const Duration(seconds: 5));
      expect(relay.published, hasLength(1));
      m.renameSection(m.store.sections.single.id, 'new-local');
      relay.holdOk!.complete();
      relay.holdOk = null;
      clock.flushMicrotasks();
      expect(names(), ['new-local']);
      clock.elapse(const Duration(seconds: 5));
      expect(relay.published, hasLength(2));
      expect(
        jsonEncode(relay.decrypt(relay.published.last)),
        contains('"new-local"'),
      );
    });

    fakeAsyncTest('a retired late OK cannot rewrite the successor cache', (
      clock,
    ) {
      relay.holdOk = Completer<void>();
      start(clock).createSection('mine');
      clock.elapse(const Duration(seconds: 5));
      final own = relay.published.single;
      final loser = relay.event(
        'channel-sections',
        blob('loser'),
        own.createdAt,
        id: ''.padLeft(64, 'f'),
      );
      relay
        ..stored.add(loser)
        ..emit(loser);
      clock.elapse(const Duration(milliseconds: 20));
      expect(names(), ['loser']);
      m.dispose(flushPending: false);
      m = ChannelSectionsManager(
        pubkey: relay.pubkey,
        relayUrl: 'https://relay.example',
        prefs: prefs(),
        crypto: ChannelSectionsCrypto(relay.keys.nsec, relay.pubkey),
        relaySession: relay.session,
        signedEventRelay: relay.signer,
        remoteEnabled: false,
        onChanged: () {},
      )..initialize();
      m.renameSection('loser', 'successor-local');
      clock.flushMicrotasks();
      final before = prefs().getString(prefs().getKeys().single);
      expect(before, contains('"successor-local"'));
      relay.holdOk!.complete();
      clock.flushMicrotasks();
      expect(names(), ['successor-local']);
      expect(prefs().getString(prefs().getKeys().single), before);
    });
  });

  group('sort', () {
    late SidebarRelay relay;
    late ChannelSortManager m;
    setUp(() => relay = SidebarRelay());
    tearDown(() => m.dispose());

    ChannelSortManager start(FakeAsync clock, {bool remote = true}) {
      m = ChannelSortManager(
        pubkey: relay.pubkey,
        relayUrl: 'wss://relay.example',
        prefs: prefs(),
        crypto: ChannelSortCrypto(relay.keys.nsec, relay.pubkey),
        relaySession: relay.session,
        signedEventRelay: relay.signer,
        remoteEnabled: remote,
        onChanged: () {},
      )..initialize();
      clock.flushMicrotasks();
      return m;
    }

    NostrEvent recent(int t) => relay.event('channel-sort', {
      'version': 1,
      'groups': {'dms': 'recent'},
    }, t);

    fakeAsyncTest('resume adopts a head the healthy socket missed', (clock) {
      start(clock);
      relay.stored.add(recent(nowSeconds()));
      m.refreshFromRelay();
      clock.flushMicrotasks();
      expect(m.sortModeFor('dms'), ChannelSortMode.recent);
      expect(relay.reqsFor('channel-sort', 'l-'), hasLength(1));
    });

    fakeAsyncTest('a retired resume read cannot overwrite its successor', (
      clock,
    ) {
      start(clock);
      relay.stored.add(recent(nowSeconds() + 9));
      final held = relay.holdHistory = Completer<void>();
      m.refreshFromRelay();
      clock.flushMicrotasks();
      m.dispose();
      start(clock, remote: false).setSortModeFor('dms', ChannelSortMode.alpha);
      clock.flushMicrotasks();
      final before = {for (final k in prefs().getKeys()) k: prefs().get(k)};
      held.complete();
      clock.flushMicrotasks();
      expect({for (final k in prefs().getKeys()) k: prefs().get(k)}, before);
    });

    fakeAsyncTest('resume leaves a pending edit alone', (clock) {
      start(clock).setSortModeFor('dms', ChannelSortMode.alpha);
      relay.stored.add(recent(nowSeconds() + 9));
      final reads = relay.reqsFor('channel-sort', 'h-').length;
      m.refreshFromRelay();
      clock.flushMicrotasks();
      expect(m.sortModeFor('dms'), ChannelSortMode.alpha);
      expect(relay.reqsFor('channel-sort', 'h-'), hasLength(reads));
    });
  });
}
