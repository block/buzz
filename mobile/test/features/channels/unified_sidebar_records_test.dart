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
    fakeAsyncTest('sections stay unpublished when the unified head arrives '
        '$arrival', (clock) {
      final relay = SidebarRelay();
      final head = relay.event('channel-workspaces', workspaces, nowSeconds());
      if (!live && !failedRead) relay.stored.add(head);
      final m = ChannelSectionsManager(
        pubkey: relay.pubkey,
        relayUrl: 'https://relay.example',
        prefs: prefs,
        crypto: ChannelSectionsCrypto(relay.keys.nsec, relay.pubkey),
        relaySession: relay.session,
        signedEventRelay: relay.signer,
        remoteEnabled: true,
        onChanged: () {},
      )..initialize();
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
      m.dispose();
      clock.flushMicrotasks();
      expect(relay.published, isEmpty);
    });

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
