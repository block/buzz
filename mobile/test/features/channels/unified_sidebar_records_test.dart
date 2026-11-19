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

  for (final live in [false, true]) {
    final arrival = live ? 'while an edit waits' : 'before any edit';
    fakeAsyncTest('sections stay unpublished when the unified head arrives '
        '$arrival', (clock) {
      final relay = SidebarRelay();
      final head = relay.event('channel-workspaces', workspaces, nowSeconds());
      if (!live) relay.stored.add(head);
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
      if (live) {
        relay
          ..stored.add(head)
          ..emit(head);
        clock.flushMicrotasks();
      }
      m
        ..createSection('later')
        ..assignChannel('general', 'launch');
      clock.elapse(const Duration(seconds: 10));

      expect(m.workspace, isNotNull);
      expect(relay.published, isEmpty);
      m.dispose();
      clock.flushMicrotasks();
      expect(relay.published, isEmpty);
    });

    fakeAsyncTest('sort stays unpublished when the unified head arrives '
        '$arrival', (clock) {
      final relay = SidebarRelay();
      final head = relay.event(
        'channel-workspace-sort',
        workspaceSort,
        nowSeconds(),
      );
      if (!live) relay.stored.add(head);
      final m = ChannelSortManager(
        pubkey: relay.pubkey,
        relayUrl: 'wss://relay.example',
        prefs: prefs,
        crypto: ChannelSortCrypto(relay.keys.nsec, relay.pubkey),
        relaySession: relay.session,
        signedEventRelay: relay.signer,
        remoteEnabled: true,
        onChanged: () {},
      )..initialize();
      clock.flushMicrotasks();
      m.setSortModeFor('dms', ChannelSortMode.recent);
      if (live) {
        relay
          ..stored.add(head)
          ..emit(head);
        clock.flushMicrotasks();
      }
      m.setSortModeFor('channels', ChannelSortMode.alpha);
      clock.elapse(const Duration(seconds: 10));

      expect(m.editable, isFalse);
      expect(m.sortModeFor('channels'), ChannelSortMode.recent);
      expect(relay.published, isEmpty);
      m.dispose();
      clock.flushMicrotasks();
      expect(relay.published, isEmpty);
    });
  }
}
