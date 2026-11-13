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

  for (final dm in [false, true]) {
    test(
      'newly muted ${dm ? 'DM' : 'channel'} cold start equals a full-window cold start',
      () async {
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        final old = message('ordinary-backlog', now() - 7200);
        final first = _container(
          prefs,
          (_SnapshotSession(['alpha'])..dm = dm)..unreadMessages.add(old),
          mutes: _StoredMutes(),
        );
        addTearDown(first.dispose);
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
        final secondSession = (_SnapshotSession(['alpha'])..dm = dm)
          ..unreadMessages.add(old)
          ..unreadMessages.addAll([
            for (var i = 0; i < 1000; i++)
              message('new-quiet-$i', now() - 1200 + i, quiet: true),
          ]);
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
          (_SnapshotSession(['alpha'])..dm = dm)
            ..unreadMessages.addAll(secondSession.unreadMessages),
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
      },
    );
  }

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
    fresh.dispose();

    // The quiet marker can itself be deleted beyond the deletion page cap.
    // Let automatic retry finish the verification and rebuild its backlog.
    await discardUnreadOnly(prefs);
    final quietHistory = [
      message('hidden-ordinary', now() - 9000),
      for (var i = 0; i < 1000; i++)
        message('old-quiet-$i', now() - 8000 + i, quiet: true),
    ];
    final seeded = _container(
      prefs,
      _SnapshotSession(['alpha'])..unreadMessages.addAll(quietHistory),
    );
    addTearDown(seeded.dispose);
    await loaded(seeded);
    expect(
      seeded
          .read(channelsProvider.notifier)
          .observedUnreadEventsByChannel['alpha']
          ?.keys,
      ['old-quiet-0'],
    );
    seeded.dispose();
    // Deleted past the cap, the verification by ID drops the marker; deleted
    // alone, the delta returns its tombstone. Either way the retry reads the
    // whole window and brings the older ordinary message back.
    final savedUnread = {
      for (final key in prefs.getKeys())
        if (key.startsWith('buzz.channel-unread.v1:'))
          key: prefs.getString(key)!,
    };
    for (final pastCap in [true, false]) {
      for (final MapEntry(:key, :value) in savedUnread.entries) {
        await prefs.setString(key, value);
      }
      final offlineHistory = [
        ...quietHistory.where((e) => e.id != 'old-quiet-0'),
        for (var i = 0; i <= (pastCap ? 1000 : 0); i++)
          NostrEvent(
            id: 'quiet-delete-$i',
            pubkey: 'alice',
            createdAt: now() - 1500 + i,
            kind: EventKind.deletion,
            tags: [
              ['h', 'alpha'],
              ['e', i == 0 ? 'old-quiet-0' : 'reaction-$i'],
            ],
            content: '',
            sig: 'sig',
          ),
      ];
      fakeAsync((clock) {
        final session = _SnapshotSession(['alpha'])
          ..unreadMessages.addAll(offlineHistory);
        final restored = _container(prefs, session);
        try {
          restored.listen(channelsProvider, (_, _) {});
          clock.flushMicrotasks();
          clock.elapse(const Duration(minutes: 1));
          clock.flushMicrotasks();
          expect(
            session.unreadFilters.any(
              (f) => f.ids?.contains('old-quiet-0') == true,
            ),
            pastCap,
          );
          expect(unread(restored), {'alpha'});
          expect(
            restored
                .read(channelsProvider.notifier)
                .observedUnreadEventsByChannel['alpha']
                ?.keys,
            contains('hidden-ordinary'),
            reason:
                'Dropping the quiet marker must schedule the full window: older ordinary backlog still exists.',
          );
        } finally {
          restored.dispose();
        }
      });
    }
  });

  for (final busyWindow in [false, true]) {
    for (final scoped in [true, false]) {
      test(
        'capped offline deletions do not resurrect a saved mention'
        '${scoped ? '' : ' (NIP-09 without h)'}'
        '${busyWindow ? ' (full message page)' : ' (another one deleted)'}',
        () async {
          SharedPreferences.setMockInitialValues({});
          final prefs = await SharedPreferences.getInstance();
          final at = now();
          final kept = message('kept-mention', at - 7300, mention: true);
          final first = _container(
            prefs,
            _SnapshotSession(['alpha'])
              ..unreadMessages.addAll([
                kept,
                message('deleted-mention', at - 7200, mention: true),
                if (!busyWindow)
                  message('also-deleted-mention', at - 7100, mention: true),
              ]),
          );
          addTearDown(first.dispose);
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
            kept,
            // A full page of newer messages that earn nothing, so no window
            // reaches back to the saved mention.
            if (busyWindow)
              for (var i = 0; i < 1000; i++)
                message('unfollowed-$i', at - 1200 + i, root: 'e' * 64),
            deletion('offline-deletion', 'deleted-mention', at - 1500),
            // Newer reaction removals fill the deletion filter's cap; without
            // the busy window, one of them deletes the other saved mention.
            for (var i = 0; i < 1000; i++)
              deletion(
                'reaction-deletion-$i',
                !busyWindow && i == 999
                    ? 'also-deleted-mention'
                    : 'reaction-$i',
                at - 1000 + i,
              ),
          ];
          var session = _SnapshotSession(['alpha'])
            ..unreadMessages.addAll(history);
          var launch = _container(prefs, session);
          addTearDown(launch.dispose);
          await loaded(launch);
          List<int?> since() => [
            for (final filter in session.unreadFilters)
              if (filter.ids == null &&
                  filter.kinds.contains(EventKind.deletion))
                filter.since,
          ];
          Iterable<String> checkedById() => [
            for (final filter in session.unreadFilters) ...?filter.ids,
          ];
          Iterable<String> observed() =>
              launch
                  .read(channelsProvider.notifier)
                  .observedUnreadEventsByChannel['alpha']
                  ?.keys ??
              const [];
          Future<void> catchUpAgain() async {
            session.unreadFilters.clear();
            await launch.read(channelsProvider.notifier).refresh();
            await launch.read(channelsProvider.notifier).waitForUnreadCatchUp();
          }

          // A delta whose full deletion page can't say the mention is still
          // there. It stays until it is checked rather than flickering.
          expect(since().single, greaterThan(0));
          expect(observed(), contains('deleted-mention'));
          // Killed before the retry, and relaunched once the burst is too old
          // for the delta to see: the saved cursor still asks to check by ID.
          launch.dispose();
          session = _SnapshotSession(['alpha'])
            ..unreadMessages.addAll(
              history.where((event) => event.kind != EventKind.deletion),
            );
          // A failed verification must survive another kill and relaunch.
          session.failIds = true;
          session.deadlineIds = !scoped;
          launch = _container(prefs, session);
          addTearDown(launch.dispose);
          await loaded(launch);
          expect(
            checkedById(),
            containsAll(['kept-mention', 'deleted-mention']),
          );
          expect(observed(), containsAll(['kept-mention', 'deleted-mention']));
          launch.dispose();
          session = _SnapshotSession(['alpha'])
            ..unreadMessages.addAll(
              history.where((event) => event.kind != EventKind.deletion),
            );
          launch = _container(prefs, session);
          addTearDown(launch.dispose);
          await loaded(launch);
          expect(
            checkedById(),
            containsAll(['kept-mention', 'deleted-mention']),
          );
          expect(observed(), ['kept-mention']);
          // Something was deleted, so the channel reads its window once (a
          // returned tombstone already made the relaunch do so), then it is
          // back to deltas, and the surviving mention stays.
          expect(since().single, busyWindow ? greaterThan(0) : 0);
          await catchUpAgain();
          expect(since().single, busyWindow ? 0 : greaterThan(0));
          for (var pass = 0; pass < 2; pass++) {
            await catchUpAgain();
            expect(since().single, greaterThan(0));
            expect(checkedById(), isEmpty);
          }
          expect(observed(), ['kept-mention']);
          launch.dispose();

          await discardUnreadOnly(prefs);
          final fresh = _container(
            prefs,
            _SnapshotSession(['alpha'])..unreadMessages.addAll(history),
          );
          addTearDown(fresh.dispose);
          await loaded(fresh);
          // A launch without saved state agrees the mention is gone.
          expect(
            fresh
                    .read(channelsProvider.notifier)
                    .observedUnreadEventsByChannel['alpha']
                    ?.keys ??
                const <String>[],
            isNot(contains('deleted-mention')),
          );
        },
      );
    }
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
