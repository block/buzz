import 'package:buzz/features/channels/channel_sections/channel_sections_manager.dart';
import 'package:buzz/features/channels/channel_sort/channel_sort_manager.dart';
import 'package:buzz/features/channels/channel_sort/channel_sort_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:shared_preferences/shared_preferences.dart';

import 'sidebar_sync_fixture.dart';

/// Desktop's unified records supersede the legacy ones. Once mobile has seen
/// one, it must never publish the legacy record: desktop would ignore it, and
/// the user would lose the edit without knowing.
void main() {
  late SharedPreferences prefs;
  setUp(() async => prefs = await freshPrefs());

  const workspaces = {
    'version': 1,
    'sections': [
      {'id': 'launch', 'name': 'Launch', 'order': 0},
    ],
    'assignments': {'general': 'launch'},
    'sectionOrder': <String>[],
  };
  const workspaceSort = {
    'version': 1,
    'groups': {'channels': 'recent'},
  };

  fakeAsyncTest('a legacy edit survives a failed read in the disposal flush', (
    clock,
  ) {
    final relay = SidebarRelay();
    relay.stored.add(
      relay.event('channel-sections', {
        'version': 1,
        'sections': [
          {'id': 'old', 'name': 'Old', 'order': 0},
        ],
        'assignments': <String, String>{},
      }, nowSeconds() - 60),
    );
    ChannelSectionsManager manager() => ChannelSectionsManager(
      pubkey: relay.pubkey,
      relayUrl: 'https://relay.example',
      prefs: prefs,
      crypto: ChannelSectionsCrypto(relay.keys.nsec, relay.pubkey),
      relaySession: relay.session,
      signedEventRelay: relay.signer,
      remoteEnabled: true,
      onChanged: () {},
    )..initialize();
    final m = manager();
    clock.flushMicrotasks();
    m.createSection('mine');
    clock.elapse(const Duration(seconds: 2));
    // The socket drops mid-debounce: the provider retires this manager and
    // its flush's first read fails.
    relay.historyFailures = 1;
    m.dispose();
    clock.elapse(const Duration(seconds: 60));
    expect(relay.published, hasLength(1));

    final replacement = manager();
    clock.flushMicrotasks();
    expect(replacement.store.sections.map((s) => s.name), ['Old', 'mine']);
    replacement.dispose();
    clock.flushMicrotasks();
  });

  for (final liveFirst in [false, true]) {
    fakeAsyncTest('an edit killed before its publish stays off desktop\'s '
        'layout${liveFirst ? ' when the lock arrives live first' : ''}', (
      clock,
    ) {
      final relay = SidebarRelay();
      ChannelSectionsManager manager() => ChannelSectionsManager(
        pubkey: relay.pubkey,
        relayUrl: 'https://relay.example',
        prefs: prefs,
        crypto: ChannelSectionsCrypto(relay.keys.nsec, relay.pubkey),
        relaySession: relay.session,
        signedEventRelay: relay.signer,
        remoteEnabled: true,
        onChanged: () {},
      )..initialize();
      // No Team record on the relay; the app dies inside the debounce.
      manager()
        ..createSection('mine')
        // Killed: nothing flushes.
        ..dispose(flushPending: false);
      clock.flushMicrotasks();
      // Desktop saves its first unified layout before mobile comes back.
      final head = relay.event('channel-workspaces', workspaces, nowSeconds());
      if (liveFirst) {
        // The startup read fails, so the head reaches mobile live first.
        relay.historyFailures = 1;
      } else {
        relay.stored.add(head);
      }
      final restarted = manager();
      clock.flushMicrotasks();
      if (liveFirst) {
        relay
          ..stored.add(head)
          ..emit(head);
        clock.elapse(const Duration(seconds: 1));
        expect(restarted.workspace, isNotNull);
        clock.elapse(const Duration(seconds: 30));
      }

      expect(restarted.workspace, isNotNull);
      expect(restarted.store.sections, isEmpty);
      expect(relay.published, isEmpty);
      restarted.dispose();
      clock.flushMicrotasks();
    });
  }

  // live: the head arrives on the subscription while an edit waits.
  // failedRead: the publish preflight fails while the relay already holds it.
  for (final (live, failedRead) in [
    (false, false),
    (true, false),
    (false, true),
  ]) {
    final arrival = failedRead
        ? 'past a failed preflight'
        : live
        ? 'while an edit waits'
        : 'before any edit';
    for (final retained in [false, true]) {
      fakeAsyncTest('sections stay unpublished when the unified head arrives '
          '$arrival${retained ? ' over a legacy head' : ''}', (clock) {
        final relay = SidebarRelay();
        final head = relay.event(
          'channel-workspaces',
          workspaces,
          nowSeconds(),
        );
        if (!live && !failedRead) relay.stored.add(head);
        if (retained) {
          relay.stored.add(
            relay.event('channel-sections', {
              'version': 1,
              'sections': [
                {'id': 'old', 'name': 'Old', 'order': 0},
              ],
              'assignments': <String, String>{},
            }, nowSeconds() - 60),
          );
        }
        ChannelSectionsManager manager({bool online = true}) =>
            ChannelSectionsManager(
              pubkey: relay.pubkey,
              relayUrl: 'https://relay.example',
              prefs: prefs,
              crypto: ChannelSectionsCrypto(relay.keys.nsec, relay.pubkey),
              relaySession: online ? relay.session : null,
              signedEventRelay: online ? relay.signer : null,
              remoteEnabled: online,
              onChanged: () {},
            )..initialize();
        final m = manager();
        clock.flushMicrotasks();
        m.createSection('mine');
        if (failedRead) {
          relay
            ..historyFailures = 1
            ..stored.add(head);
        }
        if (live) {
          relay
            ..stored.add(head)
            ..emit(head);
          clock.flushMicrotasks();
        }
        if (!failedRead) {
          m
            ..createSection('later')
            ..assignChannel('general', 'launch');
        }
        clock.elapse(const Duration(seconds: 60));

        expect(m.workspace, isNotNull);
        expect(relay.published, isEmpty);
        // Desktop imports only what the relay holds, so the unsent edits are
        // gone here too, and after a restart.
        final confirmed = [if (retained) 'Old'];
        expect(m.store.sections.map((s) => s.name), confirmed);
        m.dispose();
        clock.flushMicrotasks();
        expect(relay.published, isEmpty);
        final restarted = manager(online: false);
        clock.flushMicrotasks();
        expect(restarted.store.sections.map((s) => s.name), confirmed);
        restarted.dispose();
      });
    }

    // For sort, the third row retains the head where only the publish
    // preflight finds it: no live EVENT ever arrives.
    final sortArrival = failedRead ? 'only in the preflight' : arrival;
    fakeAsyncTest('sort stays unpublished when the unified head arrives '
        '$sortArrival', (clock) {
      final relay = SidebarRelay();
      final head = relay.event(
        'channel-workspace-sort',
        workspaceSort,
        nowSeconds(),
      );
      if (!live && !failedRead) relay.stored.add(head);
      var notified = 0;
      final m = ChannelSortManager(
        pubkey: relay.pubkey,
        relayUrl: 'wss://relay.example',
        prefs: prefs,
        crypto: ChannelSortCrypto(relay.keys.nsec, relay.pubkey),
        relaySession: relay.session,
        signedEventRelay: relay.signer,
        remoteEnabled: true,
        onChanged: () => notified++,
      )..initialize();
      clock.flushMicrotasks();
      m.setSortModeFor('dms', ChannelSortMode.recent);
      if (failedRead) relay.stored.add(head);
      var notifiedBeforeHead = notified;
      if (live) {
        relay
          ..stored.add(head)
          ..emit(head);
        clock.flushMicrotasks();
      }
      m.setSortModeFor('channels', ChannelSortMode.alpha);
      if (failedRead) notifiedBeforeHead = notified;
      clock.elapse(const Duration(seconds: 10));

      expect(m.editable, isFalse);
      expect(m.sortModeFor('channels'), ChannelSortMode.recent);
      // Home must hear about the head, whichever read found it.
      if (live || failedRead) {
        expect(notified, greaterThan(notifiedBeforeHead));
      }
      expect(relay.published, isEmpty);
      m.dispose();
      clock.flushMicrotasks();
      expect(relay.published, isEmpty);
    });
  }
}
