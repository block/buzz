part of 'channel_snapshot_test.dart';

// Each case compares a restored launch with a full-window one.
// ChannelsNotifier, storage, thread follows, and highlight computation are real.
// Relay query results obey kinds/since/ids, per-filter limits, and union dedup.
void _restoredLaunchCases() {
  int now() => DateTime.now().millisecondsSinceEpoch ~/ 1000;

  NostrEvent message(
    String id,
    int at, {
    String? root,
    bool quiet = false,
    bool mention = false,
  }) => NostrEvent(
    id: id,
    pubkey: 'alice',
    createdAt: at,
    kind: EventKind.streamMessage,
    tags: [
      ['h', 'alpha'],
      if (mention) ['p', 'me'],
      if (quiet) ['audience', 'agents'],
      if (root != null) ...[
        ['e', root, '', 'root'],
        ['e', root, '', 'reply'],
      ],
    ],
    content: 'message',
    sig: 'sig',
  );

  Future<void> loaded(ProviderContainer container) async {
    await container.read(channelsProvider.future);
    await container.read(channelsProvider.notifier).waitForUnreadCatchUp();
  }

  Set<String> unread(ProviderContainer container) => computeUnreadChannelIds(
    channels: container.read(channelsProvider).requireValue,
    readState: const ReadStateState(
      isReady: true,
      pubkey: 'me',
      contexts: {'alpha': 0},
      version: 1,
    ),
    channelsNotifier: container.read(channelsProvider.notifier),
  );

  Future<void> discardUnreadOnly(SharedPreferences prefs) async {
    for (final key in prefs.getKeys().toList()) {
      if (key.startsWith('buzz.channel-unread.v1:')) await prefs.remove(key);
    }
  }

  test('newly muted cold start equals a full-window cold start', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final old = message('ordinary-backlog', now() - 7200);
    final first = _container(
      prefs,
      _SnapshotSession(['alpha'])..unreadMessages.add(old),
      mutes: _StoredMutes(),
    );
    await loaded(first);
    expect(unread(first), {'alpha'});
    first.dispose();

    // Persisted mute changed since the saved catch-up (e.g. mute then quit).
    await ChannelMutesStorage(prefs).write(
      'me',
      'https://a.example',
      const ChannelMuteStore(
        channels: {'alpha': ChannelMuteEntry(muted: true, updatedAt: 1)},
      ),
    );
    final secondSession = _SnapshotSession(['alpha'])..unreadMessages.add(old);
    final second = _container(prefs, secondSession, mutes: _StoredMutes());
    await loaded(second);
    // The cursor notices the mute change and requests the full window.
    expect(
      secondSession.unreadFilters.every((filter) => filter.since == 0),
      isTrue,
    );
    final cachedUnread = unread(second);
    second.dispose();

    await discardUnreadOnly(prefs);
    final fresh = _container(
      prefs,
      _SnapshotSession(['alpha'])..unreadMessages.add(old),
      mutes: _StoredMutes(),
    );
    addTearDown(fresh.dispose);
    await loaded(fresh);
    expect(unread(fresh), isEmpty);
    expect(
      cachedUnread,
      unread(fresh),
      reason:
          'Saved ordinary backlog must not keep a muted channel unread after a full catch-up.',
    );
  });

  test('newly followed old reply is not skipped on cold start', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final root = 'a' * 64;
    final reply = message('older-reply', now() - 7200, root: root);
    final first = _container(
      prefs,
      _SnapshotSession(['alpha'])..unreadMessages.add(reply),
    );
    await loaded(first);
    expect(unread(first), isEmpty);
    // This is also reachable from a root message's Follow thread action;
    // opening/reading its replies is not required to follow it.
    first.read(threadFollowsProvider.notifier).followThread(root);
    first.dispose();

    final second = _container(
      prefs,
      _SnapshotSession(['alpha'])..unreadMessages.add(reply),
    );
    await loaded(second);
    final cachedUnread = unread(second);
    final cachedActivity = second
        .read(channelsProvider.notifier)
        .observedThreadReplies(second.read(channelsProvider).requireValue)
        .map((event) => event.id)
        .toList();
    second.dispose();

    await discardUnreadOnly(prefs);
    final fresh = _container(
      prefs,
      _SnapshotSession(['alpha'])..unreadMessages.add(reply),
    );
    addTearDown(fresh.dispose);
    await loaded(fresh);
    expect(unread(fresh), {'alpha'});
    expect(
      fresh
          .read(channelsProvider.notifier)
          .observedThreadReplies(fresh.read(channelsProvider).requireValue)
          .map((event) => event.id),
      ['older-reply'],
    );
    expect(cachedUnread, {'alpha'});
    expect(
      cachedActivity,
      ['older-reply'],
      reason:
          'The cursor predates the follow and has no evidence for its older replies.',
    );
  });

  test('overlapping hydration reply does not shrink the quiet window', () async {
    SharedPreferences.setMockInitialValues({});
    final prefs = await SharedPreferences.getInstance();
    final root = 'a' * 64;
    final at = now() - 1200;
    final history = [
      for (var i = 0; i < 999; i++) message('quiet-$i', at + i, quiet: true),
      message('followed-reply', at + 1000, root: root),
    ];
    final first = _container(
      prefs,
      _SnapshotSession(['alpha'])..unreadMessages.addAll(history),
    );
    first.read(threadFollowsProvider.notifier).followThread(root);
    await loaded(first);
    expect(
      first
          .read(channelsProvider.notifier)
          .observedUnreadEventsByChannel['alpha']
          ?.keys,
      ['followed-reply'],
    );
    // The stored reply remains in the unread snapshot, but no longer merits attention.
    first.read(threadFollowsProvider.notifier).unfollowThread(root);
    first.dispose();

    final second = _container(
      prefs,
      _SnapshotSession(['alpha'])..unreadMessages.addAll(history),
    );
    await loaded(second);
    final cachedUnread = unread(second);
    second.dispose();

    await discardUnreadOnly(prefs);
    final fresh = _container(
      prefs,
      _SnapshotSession(['alpha'])..unreadMessages.addAll(history),
    );
    addTearDown(fresh.dispose);
    await loaded(fresh);
    expect(
      unread(fresh),
      {'alpha'},
      reason:
          'A full 1000-message window must retain the quiet backlog marker.',
    );
    expect(
      cachedUnread,
      unread(fresh),
      reason:
          'The reply matched both the delta and ID filters: excluding it from the window hides the channel.',
    );
  });

  for (final scoped in [true, false]) {
    test('capped offline deletions do not resurrect a saved mention'
        '${scoped ? '' : ' (NIP-09 without h)'}', () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final at = now();
      final first = _container(
        prefs,
        _SnapshotSession(['alpha'])
          ..unreadMessages.add(
            message('deleted-mention', at - 7200, mention: true),
          ),
      );
      await loaded(first);
      expect(unread(first), {'alpha'});
      first.dispose();

      NostrEvent deletion(String id, String target, int time) => NostrEvent(
        id: id,
        pubkey: 'alice',
        createdAt: time,
        kind: EventKind.deletion,
        tags: [
          if (scoped) ['h', 'alpha'],
          ['e', target],
        ],
        content: '',
        sig: 'sig',
      );
      final history = [
        deletion('offline-deletion', 'deleted-mention', at - 1500),
        // Newer reaction removals fill the same deletion-kind filter's cap.
        for (var i = 0; i < 1000; i++)
          deletion('reaction-deletion-$i', 'reaction-$i', at - 1000 + i),
      ];
      final secondSession = _SnapshotSession(['alpha'])
        ..unreadMessages.addAll(history);
      final second = _container(prefs, secondSession);
      addTearDown(second.dispose);
      await loaded(second);
      List<int?> since() => [
        for (final filter in secondSession.unreadFilters)
          if (filter.ids == null && filter.kinds.contains(EventKind.deletion))
            filter.since,
      ];
      // A delta, whose full deletion page can't say the mention is still
      // there. It stays until the full window answers rather than flickering.
      expect(since().single, greaterThan(0));
      expect(unread(second), {'alpha'});
      // The retry reads the whole window, which no longer has the mention.
      secondSession.unreadFilters.clear();
      await second.read(channelsProvider.notifier).refresh();
      await second.read(channelsProvider.notifier).waitForUnreadCatchUp();
      expect(since(), [0]);
      final cachedUnread = unread(second);
      // That pass ignores its own full deletion page, so the next is a delta.
      secondSession.unreadFilters.clear();
      await second.read(channelsProvider.notifier).refresh();
      await second.read(channelsProvider.notifier).waitForUnreadCatchUp();
      expect(since().single, greaterThan(0));
      second.dispose();

      await discardUnreadOnly(prefs);
      final fresh = _container(
        prefs,
        _SnapshotSession(['alpha'])..unreadMessages.addAll(history),
      );
      addTearDown(fresh.dispose);
      await loaded(fresh);
      expect(unread(fresh), isEmpty);
      expect(
        cachedUnread,
        unread(fresh),
        reason:
            'A deletion outside the newest 1000 tombstones must not leave an absent old mention unread.',
      );
    });
  }

  test(
    'a restored thread reply lights its channel only once Activity can show it',
    () async {
      SharedPreferences.setMockInitialValues({});
      final prefs = await SharedPreferences.getInstance();
      final root = 'c' * 64;
      final reply = message('followed-reply', now() - 7200, root: root);
      final first = _container(
        prefs,
        _SnapshotSession(['alpha'])..unreadMessages.add(reply),
      );
      addTearDown(first.dispose);
      first.read(threadFollowsProvider.notifier).followThread(root);
      await loaded(first);
      expect(unread(first), {'alpha'});
      first.dispose();

      final secondSession = _SnapshotSession(['alpha'])
        ..unreadMessages.add(reply)
        ..unreadGate = Completer();
      final second = _container(prefs, secondSession);
      addTearDown(second.dispose);
      await second.read(channelsProvider.future);
      await _settle();
      // Restored without its payload while the by-ID fetch is out: nothing in
      // Activity yet, so no dot either.
      expect(secondSession.unreadFilters, isNotEmpty);
      expect(unread(second), isEmpty);
      secondSession.unreadGate!.complete();
      await second.read(channelsProvider.notifier).waitForUnreadCatchUp();
      expect(unread(second), {'alpha'});
      expect(
        second
            .read(channelsProvider.notifier)
            .observedThreadReplies(second.read(channelsProvider).requireValue)
            .map((event) => event.id),
        ['followed-reply'],
      );
    },
  );
}

/// Only supplies the preference state input to the unread pipeline; this test
/// does not exercise mute sync. The store uses the production storage format.
class _StoredMutes extends ChannelMutesNotifier {
  @override
  ChannelMutesState build() => ChannelMutesState(
    isReady: true,
    store: ChannelMutesStorage(
      ref.read(savedPrefsProvider),
    ).read('me', 'https://a.example'),
  );
}
