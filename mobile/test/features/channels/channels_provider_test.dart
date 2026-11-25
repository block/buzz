import 'dart:async';
import 'dart:convert';
import 'dart:math';
import 'dart:io';
import 'dart:isolate';
import 'dart:ui' as ui;

import 'package:fake_async/fake_async.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart' show listEquals;
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:shared_preferences_platform_interface/shared_preferences_platform_interface.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/features/activity/activity_page.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:buzz/shared/channels/open_channel_directory_storage.dart';
import 'package:buzz/shared/channels/saved_message_store.dart';
import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channel_mutes/channel_mutes_provider.dart';
import 'package:buzz/features/channels/channel_mutes/channel_mutes_storage.dart';
import 'package:buzz/features/channels/channel_unread.dart';
import 'package:buzz/features/channels/channel_filter/channel_filter_activity.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/saved_channel_page.dart';
import 'package:buzz/features/channels/thread_follows/thread_follows_provider.dart';
import 'package:buzz/features/channels/unread_badge/observed_unread_event.dart';
import 'package:buzz/features/channels/unread_badge/unread_badge_provider.dart';
import 'package:buzz/shared/read_state/read_state_format.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/features/activity/activity_provider.dart';
import 'package:buzz/features/activity/inbox_item.dart';
import 'package:buzz/features/activity/inbox_read_state.dart';
import 'package:nostr/nostr.dart' as nostr;

part 'channels_provider_live_cases.dart';
part 'channels_provider_huddle_cases.dart';
part 'channels_provider_deletion_cases.dart';
part 'channels_provider_terminal_cases.dart';
part 'channels_provider_readiness_cases.dart';
part 'channels_provider_request_path_cases.dart';
part 'channels_provider_activity_cases.dart';
part 'channels_provider_join_cases.dart';
part 'channels_provider_follow_cases.dart';
part 'channels_provider_saved_directory_cases.dart';
part 'channels_provider_thread_interest_cases.dart';
part 'channels_provider_unread_revision_cases.dart';

/// Tests for [ChannelsNotifier] in the pure-Nostr world.
///
/// The provider loads membership-backed channels first:
///   1. paginated kind:39002 memberships tagged `#p:<my-pubkey>`
///   2. kind:39000 metadata for those channel ids
/// then layers chunked live subscriptions on the `#h` tag. Browse channels
/// separately triggers paginated kind:39000 open-channel discovery.
///
/// Tests stub out the relay session by overriding [relaySessionProvider] with
/// a [_FakeRelaySession] that returns canned events from [fetchHistory] and
/// records [subscribe] calls so we can assert filter shapes and emit live
/// events on demand.
void main() {
  _huddleNavigationCases();
  _unreadReadinessCases();
  _unreadDeletionCases();
  _unreadRevisionCases();
  _requestPathCases();
  _threadActivityCases();
  _confirmedJoinCases();
  _threadFollowCases();
  _savedDirectoryCases();
  _threadInterestCases();
  const myPk = 'me';

  test('unread readiness does not wait on activity signature checks', () async {
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final mention = _signedPost(
      now - 5,
      content: 'mention',
      key: _aliceKey,
      tags: [
        ['p', _activityPk],
      ],
    );
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, _activityPk)],
      metadata: [_meta(id: _channelA, name: 'general')],
      recentMessages: [mention],
    )..pauseNextUnreadCatchUpQuery();
    final container = _buildContainer(
      session: session,
      overrides: [_testPubkeyProvider.overrideWith(_ActivityPubkey.new)],
    );
    addTearDown(container.dispose);
    await container.read(channelsProvider.future);
    final notifier = container.read(channelsProvider.notifier);
    await session.nextUnreadCatchUpQueryStarted;
    final prior = debugRunActivityChecks;
    final started = Completer<void>();
    final release = Completer<void>();
    debugRunActivityChecks = (checks) async {
      started.complete();
      await release.future;
      return checks();
    };
    try {
      var ready = false;
      final presentation = notifier.waitForUnreadCatchUp().then(
        (_) => ready = true,
      );
      session.resumePausedUnreadCatchUpQuery();
      await started.future;
      await _settle();
      final readyDuringCheck = ready;
      final unreadDuringCheck =
          notifier.observedUnreadEventsByChannel[_channelA]?.containsKey(
            mention.id,
          ) ??
          false;
      release.complete();
      await presentation;
      expect(
        notifier.observedUnreadEventsByChannel[_channelA]?.keys,
        contains(mention.id),
      );
      expect((readyDuringCheck, unreadDuringCheck), (true, true));
    } finally {
      debugRunActivityChecks = prior;
      if (!release.isCompleted) release.complete();
    }
  });

  test(
    'a list read cannot restore a latest post deleted during its signature checks',
    () async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final old = _signedPost(now - 10 * 86400, content: 'old');
      final newest = _signedPost(now - 5, content: 'newest');
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, _activityPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
        recentMessages: [old, newest],
        enforceHistoryLimits: true,
      );
      final container = _buildContainer(
        session: session,
        overrides: [
          _testPubkeyProvider.overrideWith(_ActivityPubkey.new),
          readStateProvider.overrideWith(_ActivityReadState.new),
        ],
      );
      addTearDown(container.dispose);
      container.listen(channelFilterActivityProvider(_channelA), (_, _) {});
      await container.read(channelsProvider.future);
      final notifier = container.read(channelsProvider.notifier);
      await notifier.waitForUnreadCatchUp();
      expect(notifier.activityFor(_channelA).at, newest.createdAt);
      (container.read(readStateProvider.notifier) as _ActivityReadState)
          .markReadThrough(_channelA, now);
      final prior = debugRunActivityChecks;
      final started = Completer<void>();
      final release = Completer<void>();
      debugRunActivityChecks = (checks) async {
        final result = checks();
        if (!started.isCompleted) {
          started.complete();
          await release.future;
        }
        return result;
      };
      try {
        final refreshing = notifier.refresh();
        await started.future;
        final before = session.queryBatches.length;
        final deletion = NostrEvent.fromJson(
          signNostrEvent(
            kind: EventKind.deletion,
            content: '',
            secretKey: _activityKey,
            createdAt: now - 1,
            tags: [
              ['h', _channelA],
              ['e', newest.id],
            ],
          ).toMap(),
        );
        session.recentMessages = [old, deletion];
        session.emit(deletion);
        await _waitUntil(() => !notifier.activityFor(_channelA).known);
        expect(session.queryBatches.length, before);
        release.complete();
        await refreshing;
        await notifier.waitForUnreadCatchUp();
        final afterRefresh = notifier.activityFor(_channelA);
        final beforeReconnect = session.queryBatches.length;
        session.setStatus(SessionStatus.reconnecting);
        session.setStatus(SessionStatus.connected);
        await _waitUntil(() => session.queryBatches.length > beforeReconnect);
        await notifier.waitForUnreadCatchUp();
        final afterReconnect = notifier.activityFor(_channelA);
        expect(
          [afterRefresh.at, afterReconnect.at],
          [isNot(newest.createdAt), old.createdAt],
        );
      } finally {
        debugRunActivityChecks = prior;
        if (!release.isCompleted) release.complete();
      }
    },
  );

  test(
    'discovers open channels for a user with zero channel memberships',
    () async {
      final session = _FakeRelaySession(
        memberships: const [],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'staff', visibility: 'private'),
          _meta(id: _channelD, name: 'DM', channelType: 'dm'),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);
      expect(session.directoryQueryFilters, isEmpty);

      await container.read(channelsProvider.notifier).retryDirectory();
      final channels = container.read(channelsProvider).requireValue;

      expect(channels, hasLength(1));
      expect(channels.single.id, _channelA);
      expect(channels.single.isMember, isFalse);
      expect(session.subscribeFilters, isEmpty);
      expect(session.directoryQueryFilters, isNotEmpty);
    },
  );

  test(
    'the saved directory serves readers across launches and stays current',
    () async {
      final root = await Directory.systemTemp.createTemp('saved-directory');
      addTearDown(() => root.delete(recursive: true));
      final file = File('${root.path}/directory.json');
      var now = DateTime.utc(2026, 1, 1);
      var session = _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'general', createdAt: 100)],
      );
      final sessions = [session];
      ProviderContainer launch() {
        final container = _buildContainer(
          session: session,
          overrides: [
            openChannelDirectoryStorageProvider.overrideWithValue(
              OpenChannelDirectoryStorage(Future.value(file)),
            ),
            channelDirectoryClockProvider.overrideWithValue(() => now),
          ],
        );
        addTearDown(container.dispose);
        return container;
      }

      // The first page of each read; a read pages on with `until`.
      List<int?> reads() => [
        for (final session in sessions)
          for (final filter in session.directoryQueryFilters)
            if (filter.until == null) filter.since,
      ];
      List<String> names(ProviderContainer container) => [
        for (final channel in container.read(channelsProvider).requireValue)
          channel.name,
      ];
      // Saving runs behind the list update, on real file I/O.
      Future<void> expectSaved(List<String> expected) async {
        List<String> saved() {
          if (!file.existsSync()) return const [];
          final json = jsonDecode(file.readAsStringSync()) as Map;
          return [
            for (final meta in (json['metas'] as List).cast<Map>())
              for (final tag in (meta['tags'] as List).cast<List>())
                if (tag.first == 'name') tag[1] as String,
          ]..sort();
        }

        for (var i = 0; i < 200 && !listEquals(saved(), expected); i++) {
          await Future<void>.delayed(const Duration(milliseconds: 10));
        }
        expect(saved(), expected);
      }

      // The first reader reads the whole directory and saves it.
      var container = launch();
      await container.read(channelsProvider.future);
      var channels = container.read(channelsProvider.notifier);
      await channels.ensureDirectoryLoaded();
      await _settle();
      expect(reads(), [null]);
      expect(names(container), ['general']);
      await expectSaved(['general']);
      expect(await file.readAsString(), isNot(contains('"sig"')));

      // More readers within a minute ask the relay nothing.
      await channels.ensureDirectoryLoaded();
      await _settle();
      expect(reads(), [null]);

      // A minute on, a reader asks only for changes, from a minute before the
      // newest saved one: a rename and a new channel.
      now = now.add(const Duration(minutes: 1));
      session.metadata = [
        _meta(id: _channelA, name: 'general-chat', createdAt: 150),
        _meta(id: _channelB, name: 'fresh', createdAt: 200),
      ];
      await channels.ensureDirectoryLoaded();
      await _settle();
      expect(reads().last, 100 - 60);
      expect(names(container), unorderedEquals(['general-chat', 'fresh']));
      await expectSaved(['fresh', 'general-chat']);

      // A catch-up that brings back only held events (the overlap always
      // returns the newest) neither rebuilds the list nor rewrites the file.
      final list = container.read(channelsProvider).requireValue;
      final stamp = DateTime(2020);
      file.setLastModifiedSync(stamp);
      now = now.add(const Duration(minutes: 1));
      await channels.ensureDirectoryLoaded();
      await _settle();
      expect(reads().last, 200 - 60);
      expect(
        identical(container.read(channelsProvider).requireValue, list),
        isTrue,
      );
      await Future<void>.delayed(const Duration(milliseconds: 50));
      expect(file.lastModifiedSync(), stamp);

      // After a relaunch the saved copy serves at once; the relay is asked only
      // for changes since the newest saved one.
      container.dispose();
      now = now.add(const Duration(minutes: 5));
      session = _FakeRelaySession(
        memberships: const [],
        metadata: session.metadata,
      );
      sessions.add(session);
      container = launch();
      await container.read(channelsProvider.future);
      channels = container.read(channelsProvider.notifier);
      final before = reads().length;
      await channels.ensureDirectoryLoaded();
      expect(names(container), unorderedEquals(['general-chat', 'fresh']));
      await _settle();
      expect(reads().skip(before), [200 - 60]);

      // A channel that turned private is never in a catch-up, so it stays
      // until the daily full read, which replaces the saved copy.
      session.metadata = [
        _meta(id: _channelA, name: 'general-chat', createdAt: 150),
      ];
      now = now.add(const Duration(minutes: 1));
      await channels.ensureDirectoryLoaded();
      await _settle();
      expect(names(container), unorderedEquals(['general-chat', 'fresh']));
      now = now.add(savedDirectoryFullReadAge);
      // A full read slower than the check interval is shared, not displaced:
      // a reader opening a minute later asks the relay nothing more.
      session.pauseNextDirectoryQuery();
      await channels.ensureDirectoryLoaded();
      await session.nextDirectoryQueryStarted;
      final held = reads().length;
      now = now.add(const Duration(minutes: 2));
      await channels.ensureDirectoryLoaded();
      await _settle();
      expect(reads().length, held);
      session.resumePausedDirectoryQuery();
      await _settle();
      expect(reads().last, isNull);
      expect(names(container), ['general-chat']);
      await expectSaved(['general-chat']);
      // The quiet reads never showed search a loading state.
      expect(
        container.read(channelDirectoryLoadStatusProvider).status,
        ChannelDirectoryLoadStatus.loaded,
      );

      // Opening a channel the relay no longer serves drops it at once, and a
      // read already in flight, whose response predates the drop, does not
      // bring it back.
      now = now.add(const Duration(minutes: 1));
      session.metadata = [
        _meta(id: _channelA, name: 'general-chat', createdAt: 150),
        _meta(id: _channelB, name: 'late', createdAt: 300),
      ];
      session.pauseNextDirectoryQuery();
      await channels.ensureDirectoryLoaded();
      await session.nextDirectoryQueryStarted;
      session.metadata = [];
      await expectLater(
        container.read(channelDetailsProvider(_channelA).future),
        throwsException,
      );
      await _settle();
      expect(names(container), isEmpty);
      session.resumePausedDirectoryQuery();
      await _settle();
      // The shared full read advanced the full-read age, so this was a
      // catch-up from the newest saved change.
      expect(reads().last, 150 - 60);
      expect(names(container), ['late']);
      await expectSaved(['late']);
    },
  );

  test(
    'ensureDirectoryLoaded reads again after the community switches away and back',
    () async {
      final session = _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'general')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      final config = container.read(relayConfigProvider.notifier);
      final communityA = container.read(relayConfigProvider).baseUrl;

      await container.read(channelsProvider.notifier).ensureDirectoryLoaded();
      expect(container.read(channelsProvider).requireValue.map((c) => c.id), [
        _channelA,
      ]);

      // Switch to B without loading its directory, then back to A. The switch
      // drops A's cached directory, so A's earlier load no longer serves it.
      config.update(baseUrl: 'https://community-b.example');
      await container.read(channelsProvider.future);
      config.update(baseUrl: communityA);
      await container.read(channelsProvider.future);
      final reads = session.directoryQueryFilters.length;

      await container.read(channelsProvider.notifier).ensureDirectoryLoaded();
      expect(session.directoryQueryFilters.length, greaterThan(reads));
      expect(container.read(channelsProvider).requireValue.map((c) => c.id), [
        _channelA,
      ]);
    },
  );

  test('a directory load keeps latest-message times', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [
        _meta(id: _channelA, name: 'general'),
        _meta(id: _channelB, name: 'discoverable'),
      ],
      recentMessages: [_signedPost(30)],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);
    bool isLatest(List<NostrFilter> batch) =>
        batch.isNotEmpty &&
        batch.every(
          (filter) =>
              filter.since == null &&
              filter.tags.containsKey('#h') &&
              (filter.limit == 1 || filter.limit == 20),
        );
    await container.read(channelsProvider.future);
    await _settle();
    final latestReads = session.queryBatches.where(isLatest).length;
    expect(latestReads, isPositive);

    await container.read(channelsProvider.notifier).retryDirectory();
    await _settle();

    expect(session.queryBatches.where(isLatest), hasLength(latestReads));
    final channels = container.read(channelsProvider).requireValue;
    expect(channels.map((channel) => channel.id), [_channelB, _channelA]);
    expect(
      channels.firstWhere((channel) => channel.id == _channelA).lastMessageAt,
      DateTime.fromMillisecondsSinceEpoch(30 * 1000, isUtc: true),
    );
  });

  for (final deadlineFirst in [false, true]) {
    _testWithClock('a directory load dates newly listed DMs'
        '${deadlineFirst ? ' after a latest-message deadline' : ''}', (clock) {
      NostrEvent message(String channelId, int at, String author) => NostrEvent(
        id: '$channelId-$at',
        pubkey: author,
        createdAt: at,
        kind: EventKind.streamMessageV2,
        tags: [
          ['h', channelId],
        ],
        content: 'hello',
        sig: 'sig',
      );
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [
          _meta(id: _channelA, name: 'Alice', channelType: 'dm'),
          _meta(id: _channelB, name: 'Bob', channelType: 'dm'),
        ],
        recentMessages: [message(_channelA, 30, 'alice')],
        enforceHistoryLimits: true,
      );
      final container = _buildContainer(session: session);
      container.listen(channelsProvider, (_, _) {});
      clock.flushMicrotasks();
      DateTime? latest(String id) => container
          .read(channelsProvider)
          .requireValue
          .singleWhere((channel) => channel.id == id)
          .lastMessageAt;

      // Another device opened DM B with an outgoing message, which unread
      // catch-up skips. Search's directory load discovers B first.
      session.memberships = [
        _membership(_channelA, myPk),
        _membership(_channelB, myPk),
      ];
      session.recentMessages = [
        message(_channelA, 30, 'alice'),
        message(_channelB, 40, myPk),
      ];
      if (deadlineFirst) {
        session.latestBatchError = RelayException(
          503,
          '{"error":"query timed out"}',
        );
      }
      container.read(channelsProvider.notifier).retryDirectory();
      clock.flushMicrotasks();
      if (deadlineFirst) {
        expect(latest(_channelB), isNull);
        // The retry after the deadline reads the whole list again.
        session.latestBatchError = null;
        clock.elapse(const Duration(seconds: 10));
      }

      expect(
        latest(_channelB),
        DateTime.fromMillisecondsSinceEpoch(40 * 1000, isUtc: true),
      );
      expect(
        latest(_channelA),
        DateTime.fromMillisecondsSinceEpoch(30 * 1000, isUtc: true),
      );
      container.dispose();
      clock.elapse(const Duration(minutes: 10));
    });
  }

  test('a directory load during the first channel load waits for it', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [
        _meta(id: _channelA, name: 'general'),
        _meta(id: _channelB, name: 'discoverable'),
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);
    final sub = container.listen(channelsProvider, (_, _) {});
    addTearDown(sub.close);

    // Search starts while the first load's member metadata is held.
    session.pauseNextMemberMetadataQuery();
    container.read(channelsProvider);
    await session.nextMemberMetadataQueryStarted;
    final directory = container
        .read(channelsProvider.notifier)
        .ensureDirectoryLoaded();
    await _settle();

    // The first load's reply arrives last. It must not replace the list.
    session.resumePausedMemberMetadataQuery();
    await directory;
    await _settle();

    final state = container.read(channelsProvider);
    expect(state.hasError, isFalse);
    expect(
      state.requireValue.map((c) => c.id),
      unorderedEquals([_channelA, _channelB]),
    );
    expect(
      container.read(channelDirectoryLoadStatusProvider).status,
      ChannelDirectoryLoadStatus.loaded,
    );
  });

  test('paginates channel discovery with a composite cursor', () async {
    final firstPage = List.generate(
      500,
      (index) => _meta(
        id: '${index.toString().padLeft(8, '0')}-0000-4000-8000-000000000000',
        name: 'channel-$index',
        createdAt: 10,
      ),
    );
    final finalChannel = _meta(
      id: '99999999-9999-4999-8999-999999999999',
      name: 'last-page',
      createdAt: 9,
    );
    final session = _FakeRelaySession(
      memberships: const [],
      metadataPages: [
        firstPage,
        [finalChannel],
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    expect(await container.read(channelsProvider.future), isEmpty);
    await container.read(channelsProvider.notifier).retryDirectory();
    final channels = container.read(channelsProvider).requireValue;

    expect(channels, hasLength(501));
    final directoryFilters = session.directoryQueryFilters;
    expect(directoryFilters, hasLength(3));
    expect(directoryFilters.first.until, isNull);
    expect(directoryFilters.first.extensions, isEmpty);
    expect(directoryFilters[1].until, firstPage.last.createdAt);
    expect(directoryFilters[1].extensions['before_id'], firstPage.last.id);
    expect(directoryFilters.last.until, finalChannel.createdAt);
    expect(directoryFilters.last.extensions['before_id'], finalChannel.id);
  });

  test(
    'paginates memberships when the relay caps responses below limit',
    () async {
      final firstPage = List.generate(
        100,
        (index) => _membership(
          '${index.toString().padLeft(8, '0')}-0000-4000-8000-000000000000',
          myPk,
        ),
      );
      final finalChannelId = '99999999-9999-4999-8999-999999999999';
      final finalMembership = _membership(finalChannelId, myPk);
      final session = _FakeRelaySession(
        memberships: const [],
        membershipPages: [
          firstPage,
          [finalMembership],
        ],
        metadata: [_meta(id: finalChannelId, name: 'last-membership')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      final channels = await container.read(channelsProvider.future);

      expect(channels.single.id, finalChannelId);
      expect(channels.single.isMember, isTrue);
      expect(session.membershipQueryFilters, hasLength(3));
      expect(session.membershipQueryFilters.first.until, isNull);
      expect(session.membershipQueryFilters.first.extensions, isEmpty);
      expect(session.membershipQueryFilters[1].until, firstPage.last.createdAt);
      expect(
        session.membershipQueryFilters[1].extensions['before_id'],
        firstPage.last.id,
      );
      expect(
        session.membershipQueryFilters.last.extensions['before_id'],
        finalMembership.id,
      );
    },
  );

  test('stops membership pagination when the relay repeats a page', () async {
    final repeatedPage = List.generate(
      500,
      (index) => _membership(
        '${index.toString().padLeft(8, '0')}-0000-4000-8000-000000000000',
        myPk,
      ),
    );
    final session = _FakeRelaySession(
      memberships: const [],
      membershipPages: [repeatedPage],
      repeatLastMembershipPage: true,
      maxMembershipPageRequests: 2,
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    expect(await container.read(channelsProvider.future), isEmpty);
    expect(session.membershipRequestCount, 2);
  });

  test('stops channel discovery when the relay repeats a full page', () async {
    final repeatedPage = List.generate(
      500,
      (index) => _meta(
        id: 'repeated-channel-$index',
        name: 'repeated-$index',
        createdAt: 10,
      ),
    );
    final session = _FakeRelaySession(
      memberships: const [],
      metadataPages: [repeatedPage],
      repeatLastMetadataPage: true,
      maxMetadataPageRequests: 2,
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    expect(await container.read(channelsProvider.future), isEmpty);
    await container.read(channelsProvider.notifier).retryDirectory();
    final channels = container.read(channelsProvider).requireValue;

    expect(channels, hasLength(500));
    expect(session.metadataPageRequestCount, 2);
  });

  test(
    'directory page-cap failure is distinct from an empty directory',
    () async {
      final session = _FakeRelaySession(
        memberships: const [],
        metadataPageBuilder: (pageIndex) => List.generate(
          500,
          (eventIndex) => _meta(
            id: 'channel-$pageIndex-$eventIndex',
            name: 'channel-$pageIndex-$eventIndex',
            createdAt: 1000 - pageIndex,
          ),
        ),
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);
      await container.read(channelsProvider.notifier).retryDirectory();
      expect(session.metadataPageRequestCount, 100);
      expect(
        container.read(channelDirectoryLoadStatusProvider).status,
        ChannelDirectoryLoadStatus.error,
      );
    },
  );

  test(
    'directory failure retains discovery while membership refreshes',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'discoverable'),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(
        (await container.read(
          channelsProvider.future,
        )).map((channel) => channel.id),
        [_channelA],
      );
      await container.read(channelsProvider.notifier).retryDirectory();
      expect(
        container
            .read(channelsProvider)
            .requireValue
            .map((channel) => channel.id),
        unorderedEquals([_channelA, _channelB]),
      );

      session.memberships = [
        _membership(_channelA, myPk),
        _membership(_channelD, myPk),
      ];
      session.metadata = [
        _meta(id: _channelA, name: 'general'),
        _meta(id: _channelB, name: 'discoverable'),
        _meta(id: _channelD, name: 'newly joined'),
      ];
      session.directoryFailures = 1;

      await container.read(channelsProvider.notifier).retryDirectory();

      final refreshed = container.read(channelsProvider).requireValue;
      expect(
        refreshed.map((channel) => channel.id),
        unorderedEquals([_channelA, _channelB, _channelD]),
      );
      expect(
        refreshed.firstWhere((channel) => channel.id == _channelD).isMember,
        isTrue,
      );
      expect(
        container.read(channelDirectoryLoadStatusProvider).status,
        ChannelDirectoryLoadStatus.error,
      );

      await container.read(channelsProvider.notifier).retryDirectory();

      expect(
        container.read(channelDirectoryLoadStatusProvider).status,
        ChannelDirectoryLoadStatus.loaded,
      );
    },
  );

  test('directory retry failure retains the current channel list', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'general')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    final initial = await container.read(channelsProvider.future);
    session.membershipFailures = 1;

    await container.read(channelsProvider.notifier).retryDirectory();

    expect(container.read(channelsProvider).requireValue, initial);
    expect(
      container.read(channelDirectoryLoadStatusProvider).status,
      ChannelDirectoryLoadStatus.error,
    );
  });

  test(
    'a directory retry clears an earlier failure before it yields',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      session.membershipFailures = 1;
      await container.read(channelsProvider.notifier).retryDirectory();
      expect(
        container.read(channelDirectoryLoadStatusProvider).status,
        ChannelDirectoryLoadStatus.error,
      );

      // A refresh that retires this attempt in the same microtask drain must
      // not leave the old error to be read as its result.
      final retry = container.read(channelsProvider.notifier).retryDirectory();
      expect(
        container.read(channelDirectoryLoadStatusProvider).status,
        ChannelDirectoryLoadStatus.idle,
      );
      await retry;
      expect(
        container.read(channelDirectoryLoadStatusProvider).status,
        ChannelDirectoryLoadStatus.loaded,
      );
    },
  );

  for (final rebuild in [false, true]) {
    test(
      '${rebuild ? 'Same-scope rebuild' : 'Ordinary refresh'} settles a superseded directory load for retry',
      () async {
        final session = _FakeRelaySession(
          memberships: [_membership(_channelA, myPk)],
          metadata: [
            _meta(id: _channelA, name: 'general'),
            _meta(id: _channelB, name: 'discoverable'),
          ],
        );
        final container = _buildContainer(session: session);
        addTearDown(container.dispose);

        expect(
          (await container.read(
            channelsProvider.future,
          )).map((channel) => channel.id),
          [_channelA],
        );

        session.pauseNextDirectoryQuery();
        final directory = container
            .read(channelsProvider.notifier)
            .retryDirectory();
        await session.nextDirectoryQueryStarted;

        if (rebuild) {
          // A same-scope configuration rebuild can overlap an in-flight
          // directory request when onboarding settles its community providers.
          final config = container.read(relayConfigProvider);
          container
              .read(relayConfigProvider.notifier)
              .update(baseUrl: config.baseUrl, nsec: config.nsec);
          await container.read(channelsProvider.future);
        } else {
          await container.read(channelsProvider.notifier).refresh();
        }

        session.resumePausedDirectoryQuery();
        await directory;
        await _settle();

        expect(
          container.read(channelDirectoryLoadStatusProvider).status,
          ChannelDirectoryLoadStatus.idle,
        );
        expect(
          container
              .read(channelsProvider)
              .requireValue
              .map((channel) => channel.id),
          [_channelA],
        );

        await container.read(channelsProvider.notifier).retryDirectory();

        expect(
          container.read(channelDirectoryLoadStatusProvider).status,
          ChannelDirectoryLoadStatus.loaded,
        );
        expect(
          container
              .read(channelsProvider)
              .requireValue
              .map((channel) => channel.id),
          unorderedEquals([_channelA, _channelB]),
        );
      },
    );
  }

  test('refresh disposed before directory settlement writes nothing', () async {
    final session = _FakeRelaySession(memberships: const []);
    final container = _buildContainer(session: session);
    await container.read(channelsProvider.future);

    final refresh = container.read(channelsProvider.notifier).refresh();
    container.dispose();

    await expectLater(refresh, completes);
  });

  test(
    'community switch discards a stale directory success from the old relay',
    () async {
      final session = _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'community-a-open')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);

      session.pauseNextDirectoryQuery();
      final staleDirectory = container
          .read(channelsProvider.notifier)
          .retryDirectory();
      await session.nextDirectoryQueryStarted;

      // Switch communities while community A's directory response is paused.
      session.memberships = [_membership(_channelB, myPk)];
      session.metadata = [_meta(id: _channelB, name: 'community-b-general')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://community-b.example');
      await container.read(channelsProvider.future);

      session.resumePausedDirectoryQuery();
      await staleDirectory;
      await _settle();

      final ids = container
          .read(channelsProvider)
          .requireValue
          .map((channel) => channel.id);
      expect(ids, isNot(contains(_channelA)));
      expect(ids, contains(_channelB));
      expect(
        container.read(channelDirectoryLoadStatusProvider).scope,
        isNot(
          channelDirectoryScope(
            'https://community-b.example',
            container.read(myPubkeyProvider),
          ),
        ),
      );
    },
  );

  test(
    'community switch discards a stale directory failure from the old relay',
    () async {
      final session = _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'community-a-open')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);

      session.pauseNextDirectoryQuery();
      final staleDirectory = container
          .read(channelsProvider.notifier)
          .retryDirectory();
      await session.nextDirectoryQueryStarted;

      session.memberships = [_membership(_channelB, myPk)];
      session.metadata = [_meta(id: _channelB, name: 'community-b-general')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://community-b.example');
      await container.read(channelsProvider.future);

      // Community A's directory request fails after the switch.
      session.directoryFailures = 1;
      session.resumePausedDirectoryQuery();
      await staleDirectory;
      await _settle();

      final ids = container
          .read(channelsProvider)
          .requireValue
          .map((channel) => channel.id);
      expect(ids, contains(_channelB));
      expect(
        container.read(channelDirectoryLoadStatusProvider).scope,
        isNot(
          channelDirectoryScope(
            'https://community-b.example',
            container.read(myPubkeyProvider),
          ),
        ),
      );
    },
  );

  test(
    'identity switch discards a stale directory success from the old identity',
    () async {
      final session = _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'first-identity-open')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);

      session.pauseNextDirectoryQuery();
      final staleDirectory = container
          .read(channelsProvider.notifier)
          .retryDirectory();
      await session.nextDirectoryQueryStarted;

      // Switch signing identity while the first identity's response is paused.
      session.memberships = [_membership(_channelB, _otherPk)];
      session.metadata = [_meta(id: _channelB, name: 'second-identity-joined')];
      container.read(_testPubkeyProvider.notifier).set(_otherPk);
      await container.read(channelsProvider.future);

      session.resumePausedDirectoryQuery();
      await staleDirectory;
      await _settle();

      final ids = container
          .read(channelsProvider)
          .requireValue
          .map((channel) => channel.id);
      expect(ids, isNot(contains(_channelA)));
      expect(ids, contains(_channelB));
    },
  );

  test(
    'identity switch discards a stale directory failure from the old identity',
    () async {
      final session = _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'first-identity-open')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);

      session.pauseNextDirectoryQuery();
      final staleDirectory = container
          .read(channelsProvider.notifier)
          .retryDirectory();
      await session.nextDirectoryQueryStarted;

      session.memberships = [_membership(_channelB, _otherPk)];
      session.metadata = [_meta(id: _channelB, name: 'second-identity-joined')];
      container.read(_testPubkeyProvider.notifier).set(_otherPk);
      await container.read(channelsProvider.future);

      session.directoryFailures = 1;
      session.resumePausedDirectoryQuery();
      await staleDirectory;
      await _settle();

      expect(
        container
            .read(channelsProvider)
            .requireValue
            .map((channel) => channel.id),
        contains(_channelB),
      );
    },
  );

  test(
    'community switch after directory success discards the late refresh',
    () async {
      final session = _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'community-a-open')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);

      // Community A now has a joined channel, so the directory-triggered
      // refresh reaches the live-subscribe step and parks there. That await is
      // AFTER the loader's own directory fence, which is the window this arm
      // covers: directory success, then retirement, then settlement.
      session.memberships = [_membership(_channelA, myPk)];
      session.pauseNextSubscribe();
      final staleRefresh = container
          .read(channelsProvider.notifier)
          .retryDirectory();
      await session.nextSubscribeStarted;

      await staleRefresh;
      await _settle();

      // The old scope's finite snapshot is allowed to publish before its live
      // subscription becomes ready. From the actual switch onward, however,
      // neither that snapshot nor its detached live work may republish.
      final emitted = <List<String>>[];
      container.listen(channelsProvider, (previous, next) {
        final value = next.asData?.value;
        if (value != null) {
          emitted.add(value.map((channel) => channel.id).toList());
        }
      });

      session.memberships = [_membership(_channelB, myPk)];
      session.metadata = [_meta(id: _channelB, name: 'community-b-general')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://community-b.example');
      await container.read(channelsProvider.future);

      session.resumePausedSubscribe();
      await _settle();

      expect(
        emitted.where((ids) => ids.contains(_channelA)),
        isEmpty,
        reason: 'community A channel emitted into community B scope: $emitted',
      );
      // The retired refresh must not leave a subscription on the old channel.
      expect(session.activeChannels, isNot(contains(_channelA)));
      // Nor may it claim the new scope's directory status as its own.
      expect(
        container.read(channelDirectoryLoadStatusProvider).scope,
        isNot(
          channelDirectoryScope(
            'https://community-b.example',
            container.read(myPubkeyProvider),
          ),
        ),
      );
    },
  );

  test(
    'community switch during a directory refresh discards a late failure',
    () async {
      final session = _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'community-a-open')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);

      // The directory refresh parks on the member-metadata query and fails there
      // after the switch. A retired failure must not
      // overwrite the new scope's list or push it into an error state.
      session.memberships = [_membership(_channelA, myPk)];
      session.pauseNextMemberMetadataQuery();
      final staleRefresh = container
          .read(channelsProvider.notifier)
          .retryDirectory();
      await session.nextMemberMetadataQueryStarted;

      session.memberships = [_membership(_channelB, myPk)];
      session.metadata = [_meta(id: _channelB, name: 'community-b-general')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://community-b.example');
      await container.read(channelsProvider.future);

      session.failClaimedMemberMetadataQuery = true;
      session.resumePausedMemberMetadataQuery();
      await staleRefresh;
      await _settle();

      final current = container.read(channelsProvider);
      expect(current.hasError, isFalse);
      final ids = current.requireValue.map((channel) => channel.id);
      expect(ids, isNot(contains(_channelA)));
      expect(ids, contains(_channelB));
      // A retired failure must not claim the new scope's directory status.
      expect(
        container.read(channelDirectoryLoadStatusProvider).scope,
        isNot(
          channelDirectoryScope(
            'https://community-b.example',
            container.read(myPubkeyProvider),
          ),
        ),
      );
    },
  );

  test(
    'identity switch during a directory refresh discards the late refresh',
    () async {
      final session = _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'first-identity-open')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);

      // The directory refresh parks on the hidden-DM query while
      // the signing identity changes and the new identity finishes its rebuild.
      session.pauseNextHiddenDmQuery();
      final staleRefresh = container
          .read(channelsProvider.notifier)
          .retryDirectory();
      await session.nextHiddenDmQueryStarted;

      session.memberships = [_membership(_channelB, _otherPk)];
      session.metadata = [_meta(id: _channelB, name: 'second-identity-joined')];
      container.read(_testPubkeyProvider.notifier).set(_otherPk);
      await container.read(channelsProvider.future);

      session.resumePausedHiddenDmQuery();
      await staleRefresh;
      await _settle();

      final ids = container
          .read(channelsProvider)
          .requireValue
          .map((channel) => channel.id);
      expect(ids, isNot(contains(_channelA)));
      expect(ids, contains(_channelB));
      expect(session.activeChannels, isNot(contains(_channelA)));
    },
  );

  test(
    'identity switch during a directory refresh discards a late failure',
    () async {
      final session = _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'first-identity-open')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);

      session.memberships = [_membership(_channelA, myPk)];
      session.pauseNextMemberMetadataQuery();
      final staleRefresh = container
          .read(channelsProvider.notifier)
          .retryDirectory();
      await session.nextMemberMetadataQueryStarted;

      session.memberships = [_membership(_channelB, _otherPk)];
      session.metadata = [_meta(id: _channelB, name: 'second-identity-joined')];
      container.read(_testPubkeyProvider.notifier).set(_otherPk);
      await container.read(channelsProvider.future);

      session.failClaimedMemberMetadataQuery = true;
      session.resumePausedMemberMetadataQuery();
      await staleRefresh;
      await _settle();

      final current = container.read(channelsProvider);
      expect(current.hasError, isFalse);
      final ids = current.requireValue.map((channel) => channel.id);
      expect(ids, isNot(contains(_channelA)));
      expect(ids, contains(_channelB));
    },
  );

  test('reads after the membership list share one round trip', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'joined-a')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    // Park all three reads. Any of them waiting on another would leave that one
    // unsent.
    final metadataGate = session.metadataGate = Completer<void>();
    session.pauseNextHiddenDmQuery();
    session.pauseNextHuddleStartQuery();
    final load = container.read(channelsProvider.future);
    await _settle();

    // The membership list's own kind:39002 read has no `#d` tag.
    final sentKinds = {
      for (final filter in session.historyFilters)
        if (!filter.kinds.contains(39002) || filter.tags['#d'] != null)
          ...filter.kinds,
    };
    expect(
      sentKinds,
      containsAll([39000, EventKind.dmVisibility, EventKind.huddleStarted]),
    );

    metadataGate.complete();
    session.resumePausedHiddenDmQuery();
    session.resumePausedHuddleStartQuery();
    expect((await load).map((channel) => channel.id), [_channelA]);
  });

  test('a newer ordinary refresh owns the installed membership list', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'joined-a')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);

    session.pauseNextHiddenDmQuery();
    final olderRefresh = container.read(channelsProvider.notifier).refresh();
    await session.nextHiddenDmQueryStarted;

    session.memberships = [_membership(_channelB, myPk)];
    session.metadata = [_meta(id: _channelB, name: 'joined-b')];
    await container.read(channelsProvider.notifier).refresh();

    session.resumePausedHiddenDmQuery();
    await olderRefresh;
    await _settle();

    expect(
      container
          .read(channelsProvider)
          .requireValue
          .map((channel) => channel.id),
      [_channelB],
      reason: 'an older ordinary refresh overwrote the newer membership list',
    );
    expect(session.activeChannels, {_channelB});
  });

  test(
    'a newer refresh owns the list over an older reconnect backstop',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'joined-a')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);

      session.pauseNextHiddenDmQuery();
      session.setStatus(SessionStatus.reconnecting);
      session.setStatus(SessionStatus.connected);
      await session.nextHiddenDmQueryStarted;

      session.memberships = [_membership(_channelB, myPk)];
      session.metadata = [_meta(id: _channelB, name: 'joined-b')];
      await container.read(channelsProvider.notifier).refresh();

      session.resumePausedHiddenDmQuery();
      await _settle();

      expect(
        container
            .read(channelsProvider)
            .requireValue
            .map((channel) => channel.id),
        [_channelB],
        reason:
            'an older reconnect backstop overwrote the newer membership list',
      );
      expect(session.activeChannels, {_channelB});
    },
  );

  test('a stale request\'s Huddle leg writes no member snapshot', () async {
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'joined-a')],
      huddleStarts: [
        NostrEvent(
          id: 'huddle-start',
          pubkey: myPk,
          createdAt: now,
          kind: EventKind.huddleStarted,
          tags: const [
            ['h', _channelA],
          ],
          content: '{"ephemeral_channel_id":"$_channelB"}',
          sig: 'sig',
        ),
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    final notifier = container.read(channelsProvider.notifier);

    // Park an older refresh on its Huddle-start query. Its membership snapshot
    // has already landed, so channel A is the list it is carrying.
    session.pauseNextHuddleStartQuery();
    final olderRefresh = notifier.refresh();
    await session.nextHuddleStartQueryStarted;

    // A newer refresh completes on a disjoint membership set.
    session.memberships = [
      _membership(_channelB, myPk, additionalPubkey: _otherPk),
    ];
    session.metadata = [_meta(id: _channelB, name: 'joined-b')];
    await notifier.refresh();

    // Release the older request. Its member-snapshot write sits AFTER the
    // Huddle leg, so if that leg is unfenced the stale snapshot lands.
    session.resumePausedHuddleStartQuery();
    await olderRefresh;
    await _settle();

    expect(
      container
          .read(channelsProvider)
          .requireValue
          .map((channel) => channel.id),
      [_channelB],
      reason: 'a stale request installed its own channel list',
    );
    expect(
      notifier.cachedMembersForChannel(_channelA),
      isEmpty,
      reason:
          'a stale request wrote a member snapshot past the Huddle leg fence',
    );
    expect(
      notifier.cachedMembersForChannel(_channelB).map((m) => m.pubkey),
      containsAll([myPk, _otherPk]),
      reason: 'the newer request\'s member snapshot was clobbered',
    );
  });

  test(
    'presentation waits for unread catch-up without delaying the channel list',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
        recentMessages: const [
          NostrEvent(
            id: 'initial-mention',
            pubkey: 'alice',
            createdAt: 50,
            kind: 9,
            tags: [
              ['h', _channelA],
              ['p', myPk],
            ],
            content: 'hello',
            sig: 'sig',
          ),
        ],
      )..pauseNextUnreadCatchUpQuery();
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      final channels = await container.read(channelsProvider.future);
      expect(channels.single.name, 'general');
      final notifier = container.read(channelsProvider.notifier);
      var ready = false;
      final presentation = notifier.waitForUnreadCatchUp().then(
        (_) => ready = true,
      );
      await session.nextUnreadCatchUpQueryStarted;
      await _settle();
      expect(ready, isFalse);
      expect(notifier.observedUnreadEventsByChannel, isEmpty);

      session.resumePausedUnreadCatchUpQuery();
      await presentation;
      expect(
        notifier.observedUnreadEventsByChannel[_channelA],
        contains('initial-mention'),
      );
      expect(ready, isTrue);
    },
  );

  test(
    'landing waits for destination unread generation after disconnect',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'Alpha')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      final notifier = container.read(channelsProvider.notifier);
      await notifier.waitForUnreadCatchUp();

      session.memberships = [_membership(_channelB, myPk)];
      session.metadata = [_meta(id: _channelB, name: 'Bravo')];
      session.pauseNextSubscribe();
      final subscribing = session.nextSubscribeStarted;
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://bravo.example');
      expect(
        (await container.read(channelsProvider.future)).single.id,
        _channelB,
      );
      await subscribing;
      var ready = false;
      final landing = notifier.waitForUnreadCatchUp().then((_) => ready = true);
      session.setStatus(SessionStatus.disconnected);
      session.resumePausedSubscribe();
      await _settle();
      expect(
        ready,
        isFalse,
        reason: 'Alpha history cannot satisfy Bravo readiness',
      );

      session.recentMessages = const [
        NostrEvent(
          id: 'bravo-mention',
          pubkey: 'alice',
          createdAt: 50,
          kind: 9,
          tags: [
            ['h', _channelB],
            ['p', myPk],
          ],
          content: 'Bravo mention',
          sig: 'sig',
        ),
      ];
      session.pauseNextUnreadCatchUpQuery();
      final catchingUp = session.nextUnreadCatchUpQueryStarted;
      session.setStatus(SessionStatus.connected);
      await catchingUp;
      await _settle();
      expect(ready, isFalse);
      session.resumePausedUnreadCatchUpQuery();
      await landing;
      expect(
        notifier.observedUnreadEventsByChannel[_channelB],
        contains('bravo-mention'),
      );
      expect(ready, isTrue);
    },
  );

  test('community switch discards a parked unread catch-up', () async {
    final session = _FakeRelaySession(
      memberships: const [],
      metadata: [_meta(id: _channelA, name: 'community-a-open')],
      recentMessages: const [
        NostrEvent(
          id: 'mention-in-a',
          pubkey: 'alice',
          createdAt: 50,
          kind: 9,
          tags: [
            ['h', _channelA],
            ['p', myPk],
          ],
          content: 'direct mention in community A',
          sig: 'sig',
        ),
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    expect(await container.read(channelsProvider.future), isEmpty);

    // The directory query succeeds and the refresh completes, but the unread
    // catch-up it kicks off stays parked across the community switch.
    session.memberships = [_membership(_channelA, myPk)];
    session.pauseNextUnreadCatchUpQuery();
    final staleRefresh = container
        .read(channelsProvider.notifier)
        .retryDirectory();
    await session.nextUnreadCatchUpQueryStarted;

    session.memberships = [_membership(_channelB, myPk)];
    session.metadata = [_meta(id: _channelB, name: 'community-b-general')];
    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://community-b.example');
    await container.read(channelsProvider.future);

    session.resumePausedUnreadCatchUpQuery();
    await staleRefresh;
    await _settle();

    final notifier = container.read(channelsProvider.notifier);
    expect(
      notifier.latestObservedByChannel,
      isNot(contains(_channelA)),
      reason:
          'community A unread state landed in community B: '
          '${notifier.latestObservedByChannel}',
    );
    expect(
      notifier.observedUnreadEventsByChannel,
      isNot(contains(_channelA)),
      reason:
          'community A unread events landed in community B: '
          '${notifier.observedUnreadEventsByChannel}',
    );
  });

  test('identity switch discards a parked unread catch-up', () async {
    final session = _FakeRelaySession(
      memberships: const [],
      metadata: [_meta(id: _channelA, name: 'first-identity-open')],
      recentMessages: const [
        NostrEvent(
          id: 'mention-in-a',
          pubkey: 'alice',
          createdAt: 50,
          kind: 9,
          tags: [
            ['h', _channelA],
            ['p', myPk],
          ],
          content: 'direct mention for the first identity',
          sig: 'sig',
        ),
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    expect(await container.read(channelsProvider.future), isEmpty);

    session.memberships = [_membership(_channelA, myPk)];
    session.pauseNextUnreadCatchUpQuery();
    final staleRefresh = container
        .read(channelsProvider.notifier)
        .retryDirectory();
    await session.nextUnreadCatchUpQueryStarted;

    session.memberships = [_membership(_channelB, _otherPk)];
    session.metadata = [_meta(id: _channelB, name: 'second-identity-joined')];
    container.read(_testPubkeyProvider.notifier).set(_otherPk);
    await container.read(channelsProvider.future);

    session.resumePausedUnreadCatchUpQuery();
    await staleRefresh;
    await _settle();

    final notifier = container.read(channelsProvider.notifier);
    expect(
      notifier.latestObservedByChannel,
      isNot(contains(_channelA)),
      reason:
          'first identity unread state landed on the second identity: '
          '${notifier.latestObservedByChannel}',
    );
    expect(
      notifier.observedUnreadEventsByChannel,
      isNot(contains(_channelA)),
      reason:
          'first identity unread events landed on the second identity: '
          '${notifier.observedUnreadEventsByChannel}',
    );
  });

  test('community switch discards a parked catch-up failure', () async {
    final session = _FakeRelaySession(
      memberships: const [],
      metadata: [_meta(id: _channelA, name: 'community-a-open')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    expect(await container.read(channelsProvider.future), isEmpty);

    session.memberships = [_membership(_channelA, myPk)];
    session.pauseNextUnreadCatchUpQuery();
    final staleRefresh = container
        .read(channelsProvider.notifier)
        .retryDirectory();
    await session.nextUnreadCatchUpQueryStarted;

    session.memberships = [_membership(_channelB, myPk)];
    session.metadata = [_meta(id: _channelB, name: 'community-b-general')];
    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://community-b.example');
    await container.read(channelsProvider.future);
    // Let community B's own refresh and its own catch-up settle first, so the
    // emissions counted below can only come from the parked community A work.
    await _settle();

    // A retired catch-up that fails must publish nothing: the trailing `state`
    // write belongs to whichever community is active now.
    final staleEmissions = <int>[];
    final subscription = container.listen(
      channelsProvider,
      (_, next) => staleEmissions.add(next.requireValue.length),
      fireImmediately: false,
    );
    addTearDown(subscription.close);

    session.failClaimedUnreadCatchUpQuery = true;
    session.resumePausedUnreadCatchUpQuery();
    await staleRefresh;
    await _settle();

    final current = container.read(channelsProvider);
    expect(current.hasError, isFalse);
    expect(current.requireValue.map((channel) => channel.id), [_channelB]);
    expect(
      staleEmissions,
      isEmpty,
      reason:
          'a retired catch-up failure republished community B provider state: '
          '$staleEmissions',
    );
  });

  test('identity switch discards a parked catch-up failure', () async {
    final session = _FakeRelaySession(
      memberships: const [],
      metadata: [_meta(id: _channelA, name: 'first-identity-open')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    expect(await container.read(channelsProvider.future), isEmpty);

    session.memberships = [_membership(_channelA, myPk)];
    session.pauseNextUnreadCatchUpQuery();
    final staleRefresh = container
        .read(channelsProvider.notifier)
        .retryDirectory();
    await session.nextUnreadCatchUpQueryStarted;

    session.memberships = [_membership(_channelB, _otherPk)];
    session.metadata = [_meta(id: _channelB, name: 'second-identity-joined')];
    container.read(_testPubkeyProvider.notifier).set(_otherPk);
    await container.read(channelsProvider.future);
    await _settle();

    final staleEmissions = <int>[];
    final subscription = container.listen(
      channelsProvider,
      (_, next) => staleEmissions.add(next.requireValue.length),
      fireImmediately: false,
    );
    addTearDown(subscription.close);

    session.failClaimedUnreadCatchUpQuery = true;
    session.resumePausedUnreadCatchUpQuery();
    await staleRefresh;
    await _settle();

    final current = container.read(channelsProvider);
    expect(current.hasError, isFalse);
    expect(current.requireValue.map((channel) => channel.id), [_channelB]);
    expect(
      staleEmissions,
      isEmpty,
      reason:
          'a retired catch-up failure republished the second identity provider '
          'state: $staleEmissions',
    );
  });

  test('an ordinary membership refresh retires an older catch-up', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'joined-a')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);

    // Same relay, same identity. Only the refresh generation separates the
    // parked catch-up from the membership list the user is looking at, which
    // is the window the post-join membership refresh opens.
    session.recentMessages = const [
      NostrEvent(
        id: 'mention-in-a',
        pubkey: 'alice',
        createdAt: 50,
        kind: 9,
        tags: [
          ['h', _channelA],
          ['p', myPk],
        ],
        content: 'direct mention in channel A',
        sig: 'sig',
      ),
    ];
    session.pauseNextUnreadCatchUpQuery();
    final staleRefresh = container.read(channelsProvider.notifier).refresh();
    await session.nextUnreadCatchUpQueryStarted;

    session.memberships = [_membership(_channelB, myPk)];
    session.metadata = [_meta(id: _channelB, name: 'joined-b')];
    await container.read(channelsProvider.notifier).refresh();
    await _settle();

    session.resumePausedUnreadCatchUpQuery();
    await staleRefresh;
    await _settle();

    final notifier = container.read(channelsProvider.notifier);
    expect(
      notifier.latestObservedByChannel,
      isNot(contains(_channelA)),
      reason:
          'a superseded ordinary refresh wrote unread state for a channel the '
          'user has left: ${notifier.latestObservedByChannel}',
    );
    expect(
      notifier.observedUnreadEventsByChannel,
      isNot(contains(_channelA)),
      reason:
          'a superseded ordinary refresh wrote unread events for a channel the '
          'user has left: ${notifier.observedUnreadEventsByChannel}',
    );
  });

  test('a newer refresh retires a parked backstop catch-up', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'joined-a')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);

    session.recentMessages = const [
      NostrEvent(
        id: 'mention-in-a',
        pubkey: 'alice',
        createdAt: 50,
        kind: 9,
        tags: [
          ['h', _channelA],
          ['p', myPk],
        ],
        content: 'direct mention in channel A',
        sig: 'sig',
      ),
    ];
    session.pauseNextUnreadCatchUpQuery();
    // Reconnecting runs the backstop refresh, which never fetches the
    // directory, so this is the path that carried no lifecycle token at all.
    session.setStatus(SessionStatus.reconnecting);
    session.setStatus(SessionStatus.connected);
    await session.nextUnreadCatchUpQueryStarted;

    session.memberships = [_membership(_channelB, myPk)];
    session.metadata = [_meta(id: _channelB, name: 'joined-b')];
    await container.read(channelsProvider.notifier).refresh();
    await _settle();

    session.resumePausedUnreadCatchUpQuery();
    await _settle();

    final notifier = container.read(channelsProvider.notifier);
    expect(
      notifier.latestObservedByChannel,
      isNot(contains(_channelA)),
      reason:
          'a superseded backstop refresh wrote unread state for a channel the '
          'user has left: ${notifier.latestObservedByChannel}',
    );
    expect(
      notifier.observedUnreadEventsByChannel,
      isNot(contains(_channelA)),
      reason:
          'a superseded backstop refresh wrote unread events for a channel the '
          'user has left: ${notifier.observedUnreadEventsByChannel}',
    );
  });

  test(
    'community switch discards a parked catch-up from an ordinary refresh',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'community-a-general')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);

      session.recentMessages = const [
        NostrEvent(
          id: 'mention-in-a',
          pubkey: 'alice',
          createdAt: 50,
          kind: 9,
          tags: [
            ['h', _channelA],
            ['p', myPk],
          ],
          content: 'direct mention in community A',
          sig: 'sig',
        ),
      ];
      session.pauseNextUnreadCatchUpQuery();
      final staleRefresh = container.read(channelsProvider.notifier).refresh();
      await session.nextUnreadCatchUpQueryStarted;

      session.memberships = [_membership(_channelB, myPk)];
      session.metadata = [_meta(id: _channelB, name: 'community-b-general')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://community-b.example');
      await container.read(channelsProvider.future);
      await _settle();

      session.resumePausedUnreadCatchUpQuery();
      await staleRefresh;
      await _settle();

      final notifier = container.read(channelsProvider.notifier);
      expect(
        notifier.latestObservedByChannel,
        isNot(contains(_channelA)),
        reason:
            'community A unread state landed in community B: '
            '${notifier.latestObservedByChannel}',
      );
    },
  );

  test(
    'identity switch discards a parked catch-up from an ordinary refresh',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'first-identity-general')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);

      session.recentMessages = const [
        NostrEvent(
          id: 'mention-in-a',
          pubkey: 'alice',
          createdAt: 50,
          kind: 9,
          tags: [
            ['h', _channelA],
            ['p', myPk],
          ],
          content: 'direct mention for the first identity',
          sig: 'sig',
        ),
      ];
      session.pauseNextUnreadCatchUpQuery();
      final staleRefresh = container.read(channelsProvider.notifier).refresh();
      await session.nextUnreadCatchUpQueryStarted;

      session.memberships = [_membership(_channelB, _otherPk)];
      session.metadata = [_meta(id: _channelB, name: 'second-identity-joined')];
      container.read(_testPubkeyProvider.notifier).set(_otherPk);
      await container.read(channelsProvider.future);
      await _settle();

      session.resumePausedUnreadCatchUpQuery();
      await staleRefresh;
      await _settle();

      final notifier = container.read(channelsProvider.notifier);
      expect(
        notifier.latestObservedByChannel,
        isNot(contains(_channelA)),
        reason:
            'first identity unread state landed on the second identity: '
            '${notifier.latestObservedByChannel}',
      );
    },
  );

  test('a disconnected community switch retires a parked catch-up', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'community-a-general')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);

    session.recentMessages = const [
      NostrEvent(
        id: 'mention-in-a',
        pubkey: 'alice',
        createdAt: 50,
        kind: 9,
        tags: [
          ['h', _channelA],
          ['p', myPk],
        ],
        content: 'direct mention in community A',
        sig: 'sig',
      ),
    ];
    session.pauseNextUnreadCatchUpQuery();
    final started = session.nextUnreadCatchUpQueryStarted;
    final staleRefresh = container.read(channelsProvider.notifier).refresh();
    await started;

    // Switch community while the session is down, so the new scope runs no
    // refresh of its own. This is the arm that shows the single generation
    // counter is sufficient: the rebuild's disposal bumps it even though no
    // new refresh does.
    session.setStatus(SessionStatus.disconnected);
    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://community-b.example');
    await _settle();

    session.resumePausedUnreadCatchUpQuery();
    await staleRefresh;
    await _settle();

    final notifier = container.read(channelsProvider.notifier);
    expect(
      notifier.latestObservedByChannel,
      isNot(contains(_channelA)),
      reason:
          'community A unread state survived a disconnected switch: '
          '${notifier.latestObservedByChannel}',
    );
  });

  test('a catch-up that records nothing does not repaint the list', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'joined-a')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);

    // No unread events to record, so the catch-up has nothing to publish.
    // The catch-up is detached, so its query starts inside the refresh: hold
    // the started future before awaiting the refresh or the one-shot slot is
    // already claimed and reset by the time we ask for it.
    session.pauseNextUnreadCatchUpQuery();
    final started = session.nextUnreadCatchUpQueryStarted;
    await container.read(channelsProvider.notifier).refresh();
    await started;
    await _settle();

    final emissions = <int>[];
    final subscription = container.listen(
      channelsProvider,
      (_, next) => emissions.add(next.requireValue.length),
      fireImmediately: false,
    );
    addTearDown(subscription.close);

    session.resumePausedUnreadCatchUpQuery();
    await _settle();

    expect(
      emissions,
      isEmpty,
      reason: 'an empty unread catch-up republished provider state: $emissions',
    );
  });

  test('community switch drops unread state recorded before it', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'community-a-general')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);

    // A live mention in community A records unread state the badges read.
    session.emit(
      const NostrEvent(
        id: 'mention-in-a',
        pubkey: 'alice',
        createdAt: 50,
        kind: 9,
        tags: [
          ['h', _channelA],
          ['p', myPk],
        ],
        content: 'direct mention in community A',
        sig: 'sig',
      ),
    );
    await _settle();
    final notifier = container.read(channelsProvider.notifier);
    expect(
      notifier.latestObservedByChannel,
      contains(_channelA),
      reason: 'precondition: community A unread state was never recorded',
    );

    session.memberships = [_membership(_channelB, myPk)];
    session.metadata = [_meta(id: _channelB, name: 'community-b-general')];
    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://community-b.example');
    await container.read(channelsProvider.future);
    await _settle();

    expect(
      notifier.latestObservedByChannel,
      isNot(contains(_channelA)),
      reason:
          'community A unread state survived into community B: '
          '${notifier.latestObservedByChannel}',
    );
    expect(
      notifier.observedUnreadEventsByChannel,
      isNot(contains(_channelA)),
      reason:
          'community A unread events survived into community B: '
          '${notifier.observedUnreadEventsByChannel}',
    );
  });

  test('reconnect backstop does not refetch the channel directory', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'general')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    final initialDirectoryRequests = session.metadataPageRequestCount;
    final initialMembershipRequests = session.membershipRequestCount;

    session.setStatus(SessionStatus.reconnecting);
    session.setStatus(SessionStatus.connected);
    await _waitUntil(
      () => session.membershipRequestCount > initialMembershipRequests,
    );
    for (var i = 0; i < 10; i++) {
      await Future<void>.delayed(Duration.zero);
    }

    expect(session.metadataPageRequestCount, initialDirectoryRequests);
  });

  group('search directory load on a loaded list', () {
    const channelC = '33333333-3333-4333-8333-333333333333';

    Future<ProviderContainer> loaded(_FakeRelaySession session) async {
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      // Let the first load's detached unread catch-up finish.
      await _settle();
      return container;
    }

    List<String> listedIds(ProviderContainer container) => container
        .read(channelsProvider)
        .requireValue
        .map((channel) => channel.id)
        .toList();

    test('reads only the directory and keeps joined channels', () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'discoverable'),
        ],
      );
      final container = await loaded(session);
      final joined = container.read(channelsProvider).requireValue.single;
      final memberships = session.membershipRequestCount;
      final histories = session.historyFilters.length;
      final batches = session.queryBatches.length;

      session.metadata = [
        _meta(id: _channelA, name: 'general'),
        _meta(
          id: _channelB,
          name: 'discoverable',
          visibility: 'private',
          createdAt: 2,
        ),
        _meta(id: channelC, name: 'new-open'),
      ];
      await container
          .read(channelsProvider.notifier)
          .retryDirectory(directoryOnly: true);

      // No roster, member metadata, hidden-DM, Huddle or message reads.
      expect(session.membershipRequestCount, memberships);
      expect(session.historyFilters, hasLength(histories));
      expect(session.queryBatches, hasLength(batches));
      expect(session.directoryQueryFilters, isNotEmpty);
      expect(listedIds(container), unorderedEquals([_channelA, channelC]));
      final listed = container.read(channelsProvider).requireValue;
      expect(listed.firstWhere((c) => c.id == _channelA), same(joined));
      expect(listed.firstWhere((c) => c.id == channelC).isMember, isFalse);
      expect(
        container.read(channelDirectoryLoadStatusProvider).status,
        ChannelDirectoryLoadStatus.loaded,
      );
    });

    test('a membership refresh during the load is kept', () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'discoverable'),
          _meta(id: _channelD, name: 'joined-elsewhere'),
        ],
      );
      final container = await loaded(session);

      session.pauseNextDirectoryQuery();
      final directory = container
          .read(channelsProvider.notifier)
          .retryDirectory(directoryOnly: true);
      await session.nextDirectoryQueryStarted;

      // Joining on another device lands while the directory is in flight.
      session.memberships = [
        _membership(_channelA, myPk),
        _membership(_channelD, myPk),
      ];
      await container.read(channelsProvider.notifier).refresh();
      expect(
        container.read(channelDirectoryLoadStatusProvider).status,
        ChannelDirectoryLoadStatus.loading,
      );

      session.resumePausedDirectoryQuery();
      await directory;
      await _settle();

      final listed = container.read(channelsProvider).requireValue;
      expect(
        listed.map((c) => c.id),
        unorderedEquals([_channelA, _channelB, _channelD]),
      );
      expect(listed.firstWhere((c) => c.id == _channelD).isMember, isTrue);
      expect(listed.firstWhere((c) => c.id == _channelB).isMember, isFalse);
      expect(
        container.read(channelDirectoryLoadStatusProvider).status,
        ChannelDirectoryLoadStatus.loaded,
      );
    });

    test(
      'a refresh that merged the directory before it landed keeps it',
      () async {
        final session = _FakeRelaySession(
          memberships: [_membership(_channelA, myPk)],
          metadata: [
            _meta(id: _channelA, name: 'general'),
            _meta(id: _channelB, name: 'discoverable'),
          ],
        );
        final container = await loaded(session);

        // The refresh merges the directory it has (none) and then waits for
        // its latest messages while the search load completes.
        final gate = Completer<void>();
        session.latestBatchGate = gate;
        final refresh = container.read(channelsProvider.notifier).refresh();
        await _waitUntil(() => session.latestBatchGate == null);
        await container
            .read(channelsProvider.notifier)
            .retryDirectory(directoryOnly: true);
        expect(listedIds(container), unorderedEquals([_channelA, _channelB]));

        gate.complete();
        await refresh;

        expect(listedIds(container), unorderedEquals([_channelA, _channelB]));
      },
    );

    test('a community switch discards the response', () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'community-a-open'),
        ],
      );
      final container = await loaded(session);
      final communityB = channelDirectoryScope(
        'https://community-b.example',
        container.read(myPubkeyProvider),
      );

      session.pauseNextDirectoryQuery();
      final directory = container
          .read(channelsProvider.notifier)
          .retryDirectory(directoryOnly: true);
      await session.nextDirectoryQueryStarted;

      session.memberships = [_membership(_channelD, myPk)];
      session.metadata = [_meta(id: _channelD, name: 'community-b-general')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://community-b.example');
      await container.read(channelsProvider.future);

      session.resumePausedDirectoryQuery();
      await directory;
      await _settle();

      expect(listedIds(container), [_channelD]);
      expect(
        container.read(channelDirectoryLoadStatusProvider).scope,
        isNot(communityB),
      );
    });

    test(
      'returning to a community during a discarded load loads again',
      () async {
        final session = _FakeRelaySession(
          memberships: [_membership(_channelA, myPk)],
          metadata: [
            _meta(id: _channelA, name: 'general'),
            _meta(id: _channelB, name: 'community-a-open'),
          ],
        );
        final container = await loaded(session);
        final communityA = container.read(relayConfigProvider).baseUrl;

        session.pauseNextDirectoryQuery();
        final directory = container
            .read(channelsProvider.notifier)
            .retryDirectory(directoryOnly: true);
        await session.nextDirectoryQueryStarted;

        container
            .read(relayConfigProvider.notifier)
            .update(baseUrl: 'https://community-b.example');
        await container.read(channelsProvider.future);
        container
            .read(relayConfigProvider.notifier)
            .update(baseUrl: communityA);
        await container.read(channelsProvider.future);
        await _settle();
        final directoryQueries = session.directoryQueryFilters.length;
        await container
            .read(channelsProvider.notifier)
            .retryDirectory(directoryOnly: true);

        expect(
          session.directoryQueryFilters.length,
          greaterThan(directoryQueries),
        );
        expect(
          container.read(channelDirectoryLoadStatusProvider).status,
          ChannelDirectoryLoadStatus.loaded,
        );

        // The old request ending later leaves the newer load's status alone.
        session.resumePausedDirectoryQuery();
        await directory;
        await _settle();
        expect(
          container.read(channelDirectoryLoadStatusProvider).status,
          ChannelDirectoryLoadStatus.loaded,
        );
      },
    );

    test(
      'returning to a community after a discarded load loads again',
      () async {
        final session = _FakeRelaySession(
          memberships: [_membership(_channelA, myPk)],
          metadata: [
            _meta(id: _channelA, name: 'general'),
            _meta(id: _channelB, name: 'community-a-open'),
          ],
        );
        final container = await loaded(session);
        final communityA = container.read(relayConfigProvider).baseUrl;

        session.pauseNextDirectoryQuery();
        final directory = container
            .read(channelsProvider.notifier)
            .retryDirectory(directoryOnly: true);
        await session.nextDirectoryQueryStarted;

        container
            .read(relayConfigProvider.notifier)
            .update(baseUrl: 'https://community-b.example');
        await container.read(channelsProvider.future);
        session.resumePausedDirectoryQuery();
        await directory;
        await _settle();

        container
            .read(relayConfigProvider.notifier)
            .update(baseUrl: communityA);
        await container.read(channelsProvider.future);
        await _settle();
        final directoryQueries = session.directoryQueryFilters.length;
        await container
            .read(channelsProvider.notifier)
            .retryDirectory(directoryOnly: true);

        expect(
          session.directoryQueryFilters.length,
          greaterThan(directoryQueries),
        );
        expect(
          container.read(channelDirectoryLoadStatusProvider).status,
          ChannelDirectoryLoadStatus.loaded,
        );
      },
    );
  });

  test('membership refresh does not refetch a loaded directory', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [
        _meta(id: _channelA, name: 'general'),
        _meta(id: _channelB, name: 'discoverable'),
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    await container.read(channelsProvider.notifier).retryDirectory();
    final directoryRequestCount = session.metadataPageRequestCount;

    await container.read(channelsProvider.notifier).refresh();

    expect(session.metadataPageRequestCount, directoryRequestCount);
    expect(
      container
          .read(channelsProvider)
          .requireValue
          .map((channel) => channel.id),
      unorderedEquals([_channelA, _channelB]),
    );
  });

  test('directory-aware refresh discovers open starter channels', () async {
    final session = _FakeRelaySession(
      memberships: const [],
      metadata: [
        _meta(id: _channelA, name: 'general'),
        _meta(id: _channelB, name: 'welcome-everyone'),
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    expect(await container.read(channelsProvider.future), isEmpty);

    await container
        .read(channelsProvider.notifier)
        .refresh(fetchDirectory: true);

    final channels = container.read(channelsProvider).requireValue;
    expect(channels.map((channel) => channel.name), [
      'general',
      'welcome-everyone',
    ]);
    expect(channels.every((channel) => channel.isMember), isFalse);
    expect(session.directoryQueryFilters, isNotEmpty);
  });

  test('deduplicates joined channels from directory discovery', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [
        _meta(id: _channelA, name: 'general'),
        _meta(id: _channelB, name: 'random'),
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    expect(
      (await container.read(
        channelsProvider.future,
      )).map((channel) => channel.id),
      [_channelA],
    );
    await container.read(channelsProvider.notifier).retryDirectory();
    final channels = container.read(channelsProvider).requireValue;

    expect(channels.map((channel) => channel.id), [_channelA, _channelB]);
    expect(channels.first.isMember, isTrue);
    expect(channels.last.isMember, isFalse);
    await _waitUntil(() => session.subscribeFilters.length == 1);
    expect(session.subscribeFilters, hasLength(1));
  });

  test(
    'seeds members from the channel-list snapshot during reconnect',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk, additionalPubkey: 'alice')],
        metadata: [_meta(id: _channelA, name: 'general')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      await container.read(channelsProvider.future);
      final memberQueryCount = session.historyFilters
          .where(
            (filter) =>
                filter.kinds.contains(39002) && filter.tags['#d'] != null,
          )
          .length;

      session.setStatus(SessionStatus.reconnecting);
      final members = await container.read(
        channelMembersProvider(_channelA).future,
      );

      expect(members.map((member) => member.pubkey), [myPk, 'alice']);
      expect(
        session.historyFilters
            .where(
              (filter) =>
                  filter.kinds.contains(39002) && filter.tags['#d'] != null,
            )
            .length,
        memberQueryCount,
      );
    },
  );

  _liveSubscriptionTests();
  _terminalSubscriptionTests();

  test('retains channel-list member snapshots for immediate reuse', () async {
    final joinedAt = DateTime.fromMillisecondsSinceEpoch(1000, isUtc: true);
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk, additionalPubkey: 'alice')],
      metadata: [_meta(id: _channelA, name: 'general')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    final members = container
        .read(channelsProvider.notifier)
        .cachedMembersForChannel(_channelA);

    expect(members, hasLength(2));
    expect(members.map((member) => member.pubkey), [myPk, 'alice']);
    expect(members.every((member) => member.joinedAt == joinedAt), isTrue);
  });

  test(
    'Active filter activity comes from the list reads and live delivery alone',
    () async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      NostrEvent message(String id, int at, {String channel = _channelA}) =>
          _signedPost(at, channel: channel, content: id);
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, _activityPk),
          _membership(_channelB, _activityPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'quiet'),
        ],
        // Self-authored activity counts; a future-dated head counts as now.
        recentMessages: [
          message('mine', now - 100),
          message('ahead', now + 900, channel: _channelB),
        ],
      );
      final container = _buildContainer(
        session: session,
        overrides: [_testPubkeyProvider.overrideWith(_ActivityPubkey.new)],
      );
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await _settle();
      AsyncValue<int?> activity(String id) =>
          container.read(channelFilterActivityProvider(id));
      final listeners = [
        for (final id in [_channelA, _channelB])
          container.listen(channelFilterActivityProvider(id), (_, _) {}),
      ];
      addTearDown(() {
        for (final listener in listeners) {
          listener.close();
        }
      });
      final reads = (
        session.historyFilters.length,
        session.queryBatches.length,
      );
      final before = container
          .read(channelsProvider)
          .requireValue
          .singleWhere((channel) => channel.id == _channelA)
          .lastMessageAt;
      expect(activity(_channelA).value, now - 100);
      expect(activity(_channelB).value, inInclusiveRange(now, now + 60));

      session.emit(message('live', now - 10));
      // A message whose signature fails verification is not activity.
      session.emit(
        NostrEvent.fromJson({
          ...message('forged', now - 1).toJson(),
          'sig': 'f' * 128,
        }),
      );
      await _settle();
      expect(activity(_channelA).value, now - 10);
      // Activity leaves unread and sort time alone, and costs no relay reads.
      expect(
        container
            .read(channelsProvider)
            .requireValue
            .singleWhere((channel) => channel.id == _channelA)
            .lastMessageAt,
        before,
      );
      expect((
        session.historyFilters.length,
        session.queryBatches.length,
      ), reads);

      // A reconnect reads only new channels' latest messages; the unread
      // catch-up it already sends covers listed ones.
      session.recentMessages = [
        ...session.recentMessages,
        message('missed', now - 5),
      ];
      final batches = session.queryBatches.length;
      session.setStatus(SessionStatus.reconnecting);
      session.setStatus(SessionStatus.connected);
      await _waitUntil(
        () => session.queryBatches
            .skip(batches)
            .any((batch) => batch.every((filter) => filter.since != null)),
      );
      await _settle();
      expect(activity(_channelA).value, now - 5);

      // A list read that never answered leaves activity unknown, not empty.
      final failed = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
      )..latestBatchError = RelayException(503, '{"error":"query timed out"}');
      final cold = _buildContainer(session: failed);
      addTearDown(cold.dispose);
      await cold.read(channelsProvider.future);
      await _settle();
      expect(
        cold.read(channelFilterActivityProvider(_channelA)).isLoading,
        isTrue,
      );
      // A live post that fails verification leaves it unknown; one that
      // passes makes it known.
      failed.emit(
        NostrEvent.fromJson({
          ...message('forged live', now - 30).toJson(),
          'sig': 'f' * 128,
        }),
      );
      await _settle();
      expect(cold.read(channelsProvider.notifier).activityFor(_channelA), (
        known: false,
        at: null,
      ));
      failed.emit(message('live', now - 40));
      await _settle();
      expect(cold.read(channelsProvider.notifier).activityFor(_channelA), (
        known: true,
        at: now - 40,
      ));

      // So does a read whose only post fails verification, and the next
      // list read asks for that channel again while an Active section shows
      // it.
      final forged = NostrEvent.fromJson({
        ...message('forged', now - 60).toJson(),
        'sig': 'f' * 128,
      });
      final unverified = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
        recentMessages: [forged],
      );
      final retried = _buildContainer(session: unverified);
      addTearDown(retried.dispose);
      retried.listen(channelFilterActivityProvider(_channelA), (_, _) {});
      await retried.read(channelsProvider.future);
      await _settle();
      final retriedNotifier = retried.read(channelsProvider.notifier);
      expect(retriedNotifier.activityFor(_channelA).known, isFalse);
      unverified.recentMessages = [message('signed', now - 60)];
      unverified.setStatus(SessionStatus.reconnecting);
      unverified.setStatus(SessionStatus.connected);
      await _waitUntil(() => retriedNotifier.activityFor(_channelA).known);
      expect(retriedNotifier.activityFor(_channelA).at, now - 60);

      // A covered channel with no messages is known to be empty.
      final empty = _buildContainer(
        session: _FakeRelaySession(
          memberships: [_membership(_channelA, myPk)],
          metadata: [_meta(id: _channelA, name: 'general')],
        ),
      );
      addTearDown(empty.dispose);
      await empty.read(channelsProvider.future);
      await _settle();
      expect(empty.read(channelsProvider.notifier).activityFor(_channelA), (
        known: true,
        at: null,
      ));
    },
  );

  test('a catch-up whose newest post fails verification leaves activity '
      'unknown rather than counting an older unchecked post', () async {
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    final old = _signedPost(now - 10 * 86400);
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, _activityPk)],
      metadata: [_meta(id: _channelA, name: 'general')],
      recentMessages: [old],
      enforceHistoryLimits: true,
    );
    final container = _buildContainer(
      session: session,
      overrides: [
        _testPubkeyProvider.overrideWith(_ActivityPubkey.new),
        readStateProvider.overrideWith(_ActivityReadState.new),
      ],
    );
    addTearDown(container.dispose);
    final listening = container.listen(
      channelFilterActivityProvider(_channelA),
      (_, _) {},
    );
    addTearDown(listening.close);
    await container.read(channelsProvider.future);
    final notifier = container.read(channelsProvider.notifier);
    await notifier.waitForUnreadCatchUp();
    expect(notifier.activityFor(_channelA), (known: true, at: old.createdAt));

    session.recentMessages = [
      old,
      _signedPost(now - 60),
      NostrEvent.fromJson({
        ..._signedPost(now - 30).toJson(),
        'sig': 'f' * 128,
      }),
    ];
    final batches = session.queryBatches.length;
    session.setStatus(SessionStatus.reconnecting);
    session.setStatus(SessionStatus.connected);
    await _waitUntil(
      () => session.queryBatches
          .skip(batches)
          .any((batch) => batch.every((filter) => filter.since != null)),
    );
    await notifier.waitForUnreadCatchUp();
    await _waitUntil(() => !notifier.activityFor(_channelA).known);
  });

  test('Active filter activity survives rebuilds, read gaps, failed catch-ups '
      'and deletions', () async {
    final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
    NostrEvent message(String id, int at) => _signedPost(at, content: id);
    final old = message('old', now - 10 * 86400);
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, _activityPk)],
      metadata: [_meta(id: _channelA, name: 'general')],
      recentMessages: [old],
      enforceHistoryLimits: true,
    );
    final savedDir = await Directory.systemTemp.createTemp('active-saved');
    addTearDown(() => savedDir.delete(recursive: true));
    final container = _buildContainer(
      session: session,
      overrides: [
        readStateProvider.overrideWith(_ActivityReadState.new),
        _testPubkeyProvider.overrideWith(_ActivityPubkey.new),
        savedMessageScopeProvider.overrideWithValue((
          communityId: 'community',
          pubkey: _activityPk,
        )),
        savedMessageStoreProvider.overrideWith(
          (ref) => SavedMessageStore(
            root: () async => savedDir,
            diagnostics: const NoopDiagnosticReporter(),
          ),
        ),
      ],
    );
    addTearDown(container.dispose);
    await container.read(channelsProvider.future);
    final notifier = container.read(channelsProvider.notifier);
    await notifier.waitForUnreadCatchUp();
    expect(notifier.activityFor(_channelA), (known: true, at: old.createdAt));
    // An Active section shows the channel, so its activity gaps are read.
    var shown = container.listen(
      channelFilterActivityProvider(_channelA),
      (_, _) {},
    );
    addTearDown(() => shown.close());

    // Rebuilding in the same community, as creating a Solo chat does,
    // reuses the list without re-reading it; its activity stays.
    container.invalidate(channelsProvider);
    await container.read(channelsProvider.future);
    await notifier.waitForUnreadCatchUp();
    expect(notifier.activityFor(_channelA), (known: true, at: old.createdAt));

    Future<void> reconnect() async {
      final batches = session.queryBatches.length;
      session.setStatus(SessionStatus.reconnecting);
      session.setStatus(SessionStatus.connected);
      await _waitUntil(
        () => session.queryBatches
            .skip(batches)
            .any((batch) => batch.every((filter) => filter.since != null)),
      );
      await notifier.waitForUnreadCatchUp();
      await _settle();
    }

    // A post that arrived while disconnected and was read on another
    // device is older than the read marker the catch-up starts after.
    final offline = message('offline', now - 30);
    session.recentMessages = [old, offline];
    (container.read(readStateProvider.notifier) as _ActivityReadState)
        .markReadThrough(_channelA, now - 15);
    await reconnect();
    expect(notifier.activityFor(_channelA).at, offline.createdAt);

    // An unread post newer than the gap makes reading the gap moot, so a
    // failing gap read is never sent.
    final mention = _signedPost(
      now - 5,
      content: 'hey',
      key: _aliceKey,
      tags: [
        ['p', _activityPk],
      ],
    );
    session.recentMessages = [old, offline, mention];
    (container.read(readStateProvider.notifier) as _ActivityReadState)
        .markReadThrough(_channelA, now - 8);
    session.gapBatchError = RelayException(503, '{"error":"query timed out"}');
    await reconnect();
    expect(notifier.activityFor(_channelA), (
      known: true,
      at: mention.createdAt,
    ));
    expect(
      notifier.observedUnreadEventsByChannel[_channelA]?.keys,
      contains(mention.id),
    );

    // A failed gap read with nothing newer leaves activity unknown.
    (container.read(readStateProvider.notifier) as _ActivityReadState)
        .markReadThrough(_channelA, now - 3);
    await reconnect();
    await _waitUntil(() => !notifier.activityFor(_channelA).known);
    session.gapBatchError = null;
    session.recentMessages = [old, offline];

    // A failed catch-up leaves activity unknown until one succeeds.
    session.messageBatchError = RelayException(
      503,
      '{"error":"query timed out"}',
    );
    await reconnect();
    expect(notifier.activityFor(_channelA).known, isFalse);
    session.messageBatchError = null;
    await notifier.refresh();
    await notifier.waitForUnreadCatchUp();
    await _waitUntil(() => notifier.activityFor(_channelA).known);
    expect(notifier.activityFor(_channelA), (
      known: true,
      at: offline.createdAt,
    ));

    // A channel with a gap to read is unknown until that read answers. A pull
    // to refresh drops the answer, and one that then fails starts no new
    // catch-up, so the channel stays unknown rather than settling on activity
    // the gap could hide. The pull can land while the unread request or the
    // gap read is in flight.
    for (final parkUnread in [true, false]) {
      session.recentMessages = [old, offline];
      await notifier.refresh();
      await notifier.waitForUnreadCatchUp();
      await _waitUntil(() => notifier.activityFor(_channelA).known);
      session.recentMessages = [old, offline, message('parked', now - 25)];
      final gapGate = Completer<void>();
      if (parkUnread) {
        session.pauseNextUnreadCatchUpQuery();
        final sent = session.nextUnreadCatchUpQueryStarted;
        session.setStatus(SessionStatus.reconnecting);
        session.setStatus(SessionStatus.connected);
        await sent;
      } else {
        session.gapBatchGate = gapGate;
        await reconnect();
        await _waitUntil(() => session.gapBatchGate == null);
      }
      expect(notifier.activityFor(_channelA).known, isFalse);
      session
        ..pauseNextMemberMetadataQuery()
        ..failClaimedMemberMetadataQuery = true;
      final failing = notifier.refresh();
      await session.nextMemberMetadataQueryStarted;
      parkUnread
          ? session.resumePausedUnreadCatchUpQuery()
          : gapGate.complete();
      await _settle();
      session.resumePausedMemberMetadataQuery();
      await failing;
      session.failClaimedMemberMetadataQuery = false;
      await _settle();
      expect(notifier.activityFor(_channelA).known, isFalse);
    }

    // A rebuild while disconnected resets the unread cursor, so the next
    // catch-up starts after the read marker; the post read meanwhile on
    // another device still counts.
    final missed = message('missed', now - 20);
    session.recentMessages = [old, offline, missed];
    session.setStatus(SessionStatus.reconnecting);
    container.invalidate(channelsProvider);
    final rebuilt = container.read(channelsProvider.future);
    (container.read(readStateProvider.notifier) as _ActivityReadState)
        .markReadThrough(_channelA, now - 12);
    await reconnect();
    await rebuilt;
    expect(notifier.activityFor(_channelA), (
      known: true,
      at: missed.createdAt,
    ));

    // Pull to refresh drops a deleted latest post.
    session.recentMessages = [old];
    await notifier.refresh();
    await notifier.waitForUnreadCatchUp();
    expect(notifier.activityFor(_channelA).at, old.createdAt);

    // A post that arrives while that read is out is newer than its answer.
    final gate = Completer<void>();
    session.latestBatchGate = gate;
    final refreshing = notifier.refresh();
    await _waitUntil(() => session.latestBatchGate == null);
    final live = message('live', now - 2);
    session.emit(live);
    await _settle();
    gate.complete();
    await refreshing;
    expect(notifier.activityFor(_channelA).at, live.createdAt);

    // Deleting the latest post forgets the channel's activity until the next
    // list read re-reads it.
    NostrEvent deletionOf(NostrEvent post, {int? at}) => NostrEvent.fromJson(
      signNostrEvent(
        kind: EventKind.deletion,
        content: '',
        secretKey: _activityKey,
        createdAt: at,
        tags: [
          ['h', _channelA],
          ['e', post.id],
        ],
      ).toMap(),
    );
    session.recentMessages = [old];
    session.emit(deletionOf(live));
    await _waitUntil(() => !notifier.activityFor(_channelA).known);
    await reconnect();
    expect(notifier.activityFor(_channelA), (known: true, at: old.createdAt));

    // So does a deletion made while offline, which the catch-up returns.
    final gone = message('gone', now - 4);
    session.emit(gone);
    await _waitUntil(
      () => notifier.activityFor(_channelA).at == gone.createdAt,
    );
    session.recentMessages = [old, deletionOf(gone)];
    await reconnect();
    expect(notifier.activityFor(_channelA).known, isFalse);
    session.recentMessages = [old];
    await reconnect();
    expect(notifier.activityFor(_channelA), (known: true, at: old.createdAt));

    // A gap read answers with its newest post. When the catch-up found that
    // post deleted, an older one in the gap may survive, so the channel stays
    // unknown.
    final kept = message('kept', now - 8);
    session.emit(kept);
    await _waitUntil(
      () => notifier.activityFor(_channelA).at == kept.createdAt,
    );
    final older = message('older', now - 7);
    final deletedInGap = message('deletedInGap', now - 6);
    session.recentMessages = [
      old,
      kept,
      older,
      deletedInGap,
      deletionOf(deletedInGap),
    ];
    (container.read(readStateProvider.notifier) as _ActivityReadState)
        .markReadThrough(_channelA, now - 4);
    await reconnect();
    expect(notifier.activityFor(_channelA).known, isFalse);
    session.recentMessages = [old, kept, older];
    await reconnect();
    expect(notifier.activityFor(_channelA), (known: true, at: older.createdAt));

    // So does a gap read whose signature checks fail to run.
    final unchecked = message('unchecked', now - 5);
    session.recentMessages = [old, kept, older, unchecked];
    final checks = debugRunActivityChecks;
    debugRunActivityChecks = (run) async {
      final verified = await checks(run);
      if (verified.contains(unchecked.id)) throw StateError('checks failed');
      return verified;
    };
    try {
      await reconnect();
      expect(notifier.activityFor(_channelA).known, isFalse);
    } finally {
      debugRunActivityChecks = checks;
    }
    await reconnect();
    expect(notifier.activityFor(_channelA), (
      known: true,
      at: unchecked.createdAt,
    ));

    // A deletion made while offline counts even when the read marker has
    // since moved past it on another device.
    final gone2 = message('gone2', now - 4);
    session.emit(gone2);
    await _waitUntil(
      () => notifier.activityFor(_channelA).at == gone2.createdAt,
    );
    session.recentMessages = [old, unchecked, deletionOf(gone2, at: now - 3)];
    (container.read(readStateProvider.notifier) as _ActivityReadState)
        .markReadThrough(_channelA, now - 2);
    await reconnect();
    expect(notifier.activityFor(_channelA).known, isFalse);
    session.recentMessages = [old, unchecked];
    await reconnect();
    expect(notifier.activityFor(_channelA), (
      known: true,
      at: unchecked.createdAt,
    ));

    // A newer post in the gap replaces the deleted one, so the channel stays
    // known.
    final gone3 = message('gone3', now - 4);
    session.emit(gone3);
    await _waitUntil(
      () => notifier.activityFor(_channelA).at == gone3.createdAt,
    );
    final after = message('after', now - 3);
    session.recentMessages = [
      old,
      unchecked,
      after,
      deletionOf(gone3, at: now - 3),
    ];
    await reconnect();
    expect(notifier.activityFor(_channelA), (known: true, at: after.createdAt));

    // So does a deletion in the same second as the latest post the read
    // marker sits on.
    final boundary = message('boundary', now - 1);
    session.emit(boundary);
    await _waitUntil(
      () => notifier.activityFor(_channelA).at == boundary.createdAt,
    );
    session.recentMessages = [
      old,
      unchecked,
      after,
      deletionOf(boundary, at: now - 1),
    ];
    (container.read(readStateProvider.notifier) as _ActivityReadState)
        .markReadThrough(_channelA, now - 1);
    // The earlier second it reads only serves the Active head: another post's
    // deletion there is not new, so a page being saved meanwhile still is.
    final regular = session.recentMessages;
    session.recentMessages = [
      old,
      unchecked,
      after,
      deletionOf(older, at: now - 1),
    ];
    final store = container.read(savedMessageStoreProvider);
    final page = (channelId: _channelA, rootId: '');
    final ticket = store.capture(
      container.read(savedMessageScopeProvider)!,
      page,
    );
    await reconnect();
    expect(store.isCurrent(ticket), isTrue);
    expect(notifier.activityFor(_channelA), (
      known: true,
      at: boundary.createdAt,
    ));
    session.recentMessages = regular;
    await reconnect();
    expect(notifier.activityFor(_channelA).known, isFalse);
    session.recentMessages = [old, unchecked, after];
    await reconnect();
    expect(notifier.activityFor(_channelA), (known: true, at: after.createdAt));

    // A channel no Active section shows skips its gap read, and reads as
    // unknown until a catch-up while it is shown.
    shown.close();
    await _settle();
    final hidden = message('hidden', now - 3);
    session.recentMessages = [old, hidden];
    (container.read(readStateProvider.notifier) as _ActivityReadState)
        .markReadThrough(_channelA, now - 1);
    final gapReads = session.queryBatches.where(_isGapBatch).length;
    await reconnect();
    expect(session.queryBatches.where(_isGapBatch), hasLength(gapReads));
    expect(notifier.activityFor(_channelA).known, isFalse);
    shown = container.listen(
      channelFilterActivityProvider(_channelA),
      (_, _) {},
    );
    await reconnect();
    await _waitUntil(
      () => notifier.activityFor(_channelA).at == hidden.createdAt,
    );

    // So does a catch-up that fails before it sends anything.
    final readState =
        container.read(readStateProvider.notifier) as _ActivityReadState;
    readState.unavailable = true;
    container.invalidate(readStateProvider);
    session.setStatus(SessionStatus.reconnecting);
    session.setStatus(SessionStatus.connected);
    await _waitUntil(() => !notifier.activityFor(_channelA).known);
  });

  for (final (catchUp, before) in [
    (true, true),
    (true, false),
    (false, false),
  ]) {
    test(
      'a latest post deleted during signature checks leaves no stale activity '
      '(catch-up: $catchUp, deleted before the read: $before)',
      () async {
        final configured = debugRunActivityChecks;
        addTearDown(() => debugRunActivityChecks = configured);
        debugRunActivityChecks = (checks) => Isolate.run(checks);
        final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final old = _signedPost(now - 10 * 86400);
        final middle = _signedPost(now - 90);
        final head = _signedPost(now - 60);
        final session = _FakeRelaySession(
          memberships: [_membership(_channelA, _activityPk)],
          metadata: [_meta(id: _channelA, name: 'general')],
          recentMessages: [old],
          enforceHistoryLimits: true,
        );
        final container = _buildContainer(
          session: session,
          overrides: [
            _testPubkeyProvider.overrideWith(_ActivityPubkey.new),
            readStateProvider.overrideWith(_ActivityReadState.new),
          ],
        );
        addTearDown(container.dispose);
        final listening = container.listen(
          channelFilterActivityProvider(_channelA),
          (_, _) {},
        );
        addTearDown(listening.close);
        await container.read(channelsProvider.future);
        final notifier = container.read(channelsProvider.notifier);
        await notifier.waitForUnreadCatchUp();
        expect(notifier.activityFor(_channelA), (
          known: true,
          at: old.createdAt,
        ));
        final sampled = Completer<void>();
        final release = Completer<void>();
        debugRunActivityChecks = (checks) async {
          final verified = await Isolate.run(checks);
          if (!before && !sampled.isCompleted && verified.contains(head.id)) {
            sampled.complete();
            await release.future;
          }
          return verified;
        };
        session.recentMessages = [old, if (catchUp) middle, head];
        Future<void>? refreshing;
        final batchesBefore = session.queryBatches.length;
        if (catchUp) {
          if (before) {
            session.pauseNextUnreadCatchUpQuery();
          } else {
            // Read on another device between the two posts, so the catch-up
            // starts after `middle` and leaves it in a gap.
            (container.read(readStateProvider.notifier) as _ActivityReadState)
                .markReadThrough(_channelA, now - 75);
          }
          session.setStatus(SessionStatus.reconnecting);
          session.setStatus(SessionStatus.connected);
        } else {
          // The refresh reads a head newer than the cached one, so deleting
          // it forgets no committed head. The other device has already read
          // through the deletion's second.
          (container.read(readStateProvider.notifier) as _ActivityReadState)
              .markReadThrough(_channelA, now);
          refreshing = notifier.refresh();
        }
        if (before) {
          await session.nextUnreadCatchUpQueryStarted;
        } else {
          await sampled.future.timeout(const Duration(seconds: 5));
        }
        final deletion = NostrEvent.fromJson(
          signNostrEvent(
            kind: EventKind.deletion,
            secretKey: _activityKey,
            content: '',
            createdAt: now,
            tags: [
              ['h', _channelA],
              ['e', head.id],
            ],
          ).toMap(),
        );
        session.recentMessages = [old, if (catchUp) middle, deletion];
        session.emit(deletion);
        await _settle();
        release.complete();
        if (before) session.resumePausedUnreadCatchUpQuery();
        if (refreshing != null) await refreshing;
        await notifier.waitForUnreadCatchUp();
        await _settle();
        if (catchUp && before) {
          await _waitForIsolate(
            () => notifier.activityFor(_channelA).at == middle.createdAt,
          );
        }
        final result = notifier.activityFor(_channelA);
        if (catchUp) {
          if (before) {
            expect(result, (known: true, at: middle.createdAt));
          } else {
            // The deleted head leaves the channel unknown, with no gap read
            // that could settle it on a post older than the deleted one.
            expect(result.known, isFalse);
            expect(
              session.queryBatches.skip(batchesBefore).where(_isGapBatch),
              isEmpty,
            );
          }
          expect(result.known && result.at == old.createdAt, isFalse);
        } else {
          expect(result.known, isFalse);
          final pre = session.queryBatches.length;
          session.setStatus(SessionStatus.reconnecting);
          session.setStatus(SessionStatus.connected);
          // The reconnect's list read checks signatures on the real isolate
          // before its catch-up goes out.
          await _waitForIsolate(
            () => session.queryBatches
                .skip(pre)
                .any((b) => b.every((f) => f.since != null)),
          );
          await notifier.waitForUnreadCatchUp();
          await _waitForIsolate(() => notifier.activityFor(_channelA).known);
          expect(notifier.activityFor(_channelA), (
            known: true,
            at: old.createdAt,
          ));
        }
      },
    );
  }

  // Past one request's channel values, the head deletions go out on their
  // own after the gap posts; below it, with them.
  for (final empty in [0, 2]) {
    test('gap posts settle Active activity even when the head deletion read '
        'fails ($empty channels without a head)', () async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final ids = [
        for (var i = 0; i <= kMaxExplicitChannelValues ~/ 2; i++) 'gap-$i',
      ];
      final heads = {
        for (final id in ids.take(ids.length - empty))
          id: _signedPost(now - 100, channel: id, content: id),
      };
      final newer = _signedPost(now - 30, channel: ids[0], content: 'newer');
      final deletion = NostrEvent.fromJson(
        signNostrEvent(
          kind: EventKind.deletion,
          content: '',
          secretKey: _activityKey,
          createdAt: now - 50,
          tags: [
            ['h', ids[1]],
            ['e', heads[ids[1]]!.id],
          ],
        ).toMap(),
      );
      final session = _FakeRelaySession(
        memberships: [for (final id in ids) _membership(id, _activityPk)],
        metadata: [for (final id in ids) _meta(id: id, name: id)],
        recentMessages: heads.values.toList(),
        enforceHistoryLimits: true,
      );
      final container = _buildContainer(
        session: session,
        overrides: [
          readStateProvider.overrideWith(_ActivityReadState.new),
          _testPubkeyProvider.overrideWith(_ActivityPubkey.new),
        ],
      );
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      final notifier = container.read(channelsProvider.notifier);
      await notifier.waitForUnreadCatchUp();
      for (final id in ids) {
        final shown = container.listen(
          channelFilterActivityProvider(id),
          (_, _) {},
        );
        addTearDown(shown.close);
      }
      Future<void> reconnect() async {
        final batches = session.queryBatches.length;
        session.setStatus(SessionStatus.reconnecting);
        session.setStatus(SessionStatus.connected);
        await _waitUntil(
          () => session.queryBatches
              .skip(batches)
              .any((batch) => batch.every((filter) => filter.since != null)),
        );
        await notifier.waitForUnreadCatchUp();
        await _settle();
      }

      // Read on another device while disconnected, so every channel has a gap.
      session.recentMessages = [...heads.values, newer, deletion];
      for (final id in ids) {
        (container.read(readStateProvider.notifier) as _ActivityReadState)
            .markReadThrough(id, now - 15);
      }
      session.headDeletionBatchError = RelayException(
        503,
        '{"error":"query timed out"}',
      );
      final before = session.queryBatches.length;
      await reconnect();
      expect(
        session.queryBatches
            .skip(before)
            .where(
              (batch) =>
                  _isGapBatch(batch) ||
                  batch.every((filter) => filter.tags.containsKey('#e')),
            ),
        hasLength(empty == 0 ? 2 : 1),
      );
      expect(notifier.activityFor(ids[0]), (known: true, at: newer.createdAt));
      expect(notifier.activityFor(ids[1]).known, isFalse);
      // Only a separate deletion read can fail on its own.
      expect(notifier.activityFor(ids[2]).known, empty > 0);

      // Once the deletions are read, only the deleted head stays unknown.
      session.headDeletionBatchError = null;
      await reconnect();
      expect(notifier.activityFor(ids[1]).known, isFalse);
      expect(notifier.activityFor(ids[2]), (
        known: true,
        at: heads[ids[2]]!.createdAt,
      ));
    });
  }

  test('live channel events update channel lastMessageAt', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'general', createdAt: 10)],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);

    // Emit a live message event on channelA.
    session.emit(
      NostrEvent(
        id: 'event-1',
        pubkey: 'alice',
        createdAt: 20,
        kind: EventKind.streamMessageV2,
        tags: const [
          ['h', _channelA],
        ],
        content: 'new message',
        sig: 'sig',
      ),
    );

    final channels = container.read(channelsProvider).value!;
    expect(channels.single.lastMessageAt?.millisecondsSinceEpoch, 20 * 1000);
  });

  group('latest-message batch failure', () {
    Future<List<NostrFilter>> fallbackFilters(Object error) async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
      )..messageBatchError = error;
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await _waitUntil(() => session.queryBatches.isNotEmpty);
      await Future<void>.delayed(Duration.zero);
      return session.historyFilters
          .where(
            (filter) =>
                filter.kinds.contains(EventKind.streamMessageV2) &&
                (filter.tags['#h']?.contains(_channelA) ?? false),
          )
          .toList();
    }

    test('a relay deadline never reaches the websocket fallback', () async {
      expect(
        await fallbackFilters(
          RelayException(503, '{"error":"query timed out"}'),
        ),
        isEmpty,
      );
    });

    test('an ordinary failure still uses the websocket fallback', () async {
      expect(await fallbackFilters(Exception('bridge down')), isNotEmpty);
    });
  });

  test('channel batches stay within the relay explicit-channel cap', () async {
    final ids = [
      for (var i = 0; i <= kMaxExplicitChannelValues; i++) 'capped-channel-$i',
    ];
    const backing = 'capped-huddle-backing';
    final session = _FakeRelaySession(
      memberships: [
        for (final id in ids) _membership(id, myPk),
        _membership(backing, myPk, ownerPubkey: myPk),
      ],
      metadata: [
        for (final id in ids) _meta(id: id, name: id),
        _meta(
          id: backing,
          name: 'huddle-backing',
          ttlSeconds: 3600,
          visibility: 'private',
        ),
      ],
      huddleStarts: [
        _huddleStart(pubkey: myPk, parent: ids.last, backing: backing),
      ],
      recentMessages: [
        NostrEvent(
          id: 'last-chunk',
          pubkey: 'alice',
          createdAt: 30,
          kind: EventKind.streamMessageV2,
          tags: [
            ['h', ids.last],
          ],
          content: 'hi',
          sig: 'sig',
        ),
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    final channels = await container.read(channelsProvider.future);
    await _waitUntil(
      () => session.queryBatches.any(
        (batch) => batch.isNotEmpty && batch.every((f) => f.since != null),
      ),
    );
    await _settle();

    final batches = session.queryBatches.where((batch) => batch.isNotEmpty);
    expect(
      batches.map(_explicitChannelValues),
      everyElement(lessThanOrEqualTo(kMaxExplicitChannelValues)),
    );
    expect(session.messageHistoryCount, 0);
    expect(
      channels.firstWhere((channel) => channel.id == ids.last).lastMessageAt,
      DateTime.fromMillisecondsSinceEpoch(30 * 1000, isUtc: true),
    );
    final huddleFilters = session.historyFilters.where(
      (filter) => filter.kinds.contains(EventKind.huddleStarted),
    );
    expect(
      huddleFilters.map((filter) => filter.tags['#h']!.length),
      everyElement(lessThanOrEqualTo(kMaxExplicitChannelValues)),
    );
    expect(channels.map((channel) => channel.id), unorderedEquals(ids));
  });

  test('channel batches keep four relay requests in flight', () async {
    final ids = [
      for (var i = 0; i <= 4 * kMaxExplicitChannelValues; i++)
        'bounded-channel-$i',
    ];
    final session = _BridgeDownRelaySession(
      memberships: [for (final id in ids) _membership(id, myPk)],
      metadata: [for (final id in ids) _meta(id: id, name: id)],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);
    // Held requests take real time, so wait on the clock, not microtasks.
    final deadline = DateTime.now().add(const Duration(seconds: 10));
    while (!session.peakInFlight.containsKey('unread') ||
        session.inFlight.values.any((open) => open > 0)) {
      if (DateTime.now().isAfter(deadline)) fail('Unread catch-up never ran');
      await Future<void>.delayed(const Duration(milliseconds: 5));
    }

    // Every HTTP chunk failed, so each batch also ran the websocket fallback.
    expect(session.peakInFlight, {'latest': 4, 'unread': 4, 'huddle': 4});
    // The fallback covered latest messages, unread messages, and deletions.
    expect(session.messageHistoryCount, 3 * ids.length);
  });

  group('deadline-unavailable channel batches', () {
    Object deadline() => RelayException(503, '{"error":"query timed out"}');
    bool isUnread(List<NostrFilter> batch) =>
        batch.isNotEmpty && batch.every((filter) => filter.since != null);
    NostrEvent message(
      String id,
      int createdAt, {
      bool mention = false,
      bool quiet = false,
    }) => NostrEvent(
      id: id,
      pubkey: 'alice',
      createdAt: createdAt,
      kind: EventKind.streamMessageV2,
      tags: [
        const ['h', _channelA],
        if (mention) const ['p', myPk],
        if (quiet) const ['audience', 'agents'],
      ],
      content: 'hi',
      sig: 'sig',
    );

    test('a latest-message deadline keeps known timestamps', () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
        recentMessages: [message('m1', 30)],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await _settle();
      session.latestBatchError = deadline();
      await container.read(channelsProvider.notifier).refresh();
      await _settle();
      expect(
        container.read(channelsProvider).value!.single.lastMessageAt,
        DateTime.fromMillisecondsSinceEpoch(30 * 1000, isUtc: true),
      );
      expect(session.messageHistoryCount, 0);
    });

    test(
      'a cold-start latest-message deadline is filled by unread catch-up',
      () async {
        final session = _FakeRelaySession(
          memberships: [_membership(_channelA, myPk)],
          metadata: [_meta(id: _channelA, name: 'general')],
          recentMessages: [message('mention', 30, mention: true)],
        )..latestBatchError = deadline();
        final container = _buildContainer(session: session);
        addTearDown(container.dispose);
        await container.read(channelsProvider.future);
        await _waitUntil(() => session.queryBatches.any(isUnread));
        await _settle();
        final notifier = container.read(channelsProvider.notifier);
        expect(notifier.observedUnreadEventsByChannel[_channelA]?.keys, [
          'mention',
        ]);
        expect(
          container.read(channelsProvider).value!.single.lastMessageAt,
          DateTime.fromMillisecondsSinceEpoch(30 * 1000, isUtc: true),
        );
        expect(session.messageHistoryCount, 0);
      },
    );

    test('unread catch-up never moves lastMessageAt backward', () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
        recentMessages: [message('older', 30, mention: true)],
      )..pauseNextUnreadCatchUpQuery();
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await session.nextUnreadCatchUpQueryStarted;
      // A newer live event lands while the catch-up is in flight.
      session.emit(message('newer', 60, mention: true));
      session.resumePausedUnreadCatchUpQuery();
      await _settle();
      expect(
        container
            .read(channelsProvider.notifier)
            .observedUnreadEventsByChannel[_channelA]
            ?.keys,
        containsAll(['older', 'newer']),
      );
      expect(
        container.read(channelsProvider).value!.single.lastMessageAt,
        DateTime.fromMillisecondsSinceEpoch(60 * 1000, isUtc: true),
      );
    });

    test(
      'a repeated catch-up starts before the last one by the relay drift',
      () async {
        int now() => DateTime.now().millisecondsSinceEpoch ~/ 1000;
        final session = _FakeRelaySession(
          memberships: [_membership(_channelA, myPk)],
          metadata: [_meta(id: _channelA, name: 'general')],
          recentMessages: [message('seen', now() - 3600, mention: true)],
        );
        final container = _buildContainer(session: session);
        addTearDown(container.dispose);
        final notifier = container.read(channelsProvider.notifier);
        List<int?> lastUnreadSince() => [
          for (final filter in session.queryBatches.lastWhere(isUnread))
            filter.since,
        ];
        Future<(int, int)> refreshed() async {
          final before = session.queryBatches.where(isUnread).length;
          final from = now();
          await notifier.refresh();
          await _waitUntil(
            () => session.queryBatches.where(isUnread).length > before,
          );
          await _settle();
          return (from, now());
        }

        final firstFrom = now();
        await container.read(channelsProvider.future);
        await _waitUntil(() => session.queryBatches.any(isUnread));
        await _settle();
        final firstTo = now();
        // Never read: the first catch-up asks for the whole window.
        expect(lastUnreadSince(), [0, 0]);

        // While the app is away, a message from a phone whose clock is slow
        // reaches the relay with a created_at well before the newest one the
        // app has seen.
        session.recentMessages.add(
          message('late', firstFrom - 1500, mention: true),
        );
        final (secondFrom, secondTo) = await refreshed();
        for (final since in lastUnreadSince()) {
          expect(
            since,
            inInclusiveRange(
              firstFrom - relayCrossDeviceSkewSeconds,
              firstTo - relayCrossDeviceSkewSeconds,
            ),
            reason: 'starts 1860s before the previous catch-up was sent',
          );
        }
        expect(
          notifier.observedUnreadEventsByChannel[_channelA]?.keys,
          unorderedEquals(['seen', 'late']),
        );

        // Each catch-up moves the start forward.
        await refreshed();
        for (final since in lastUnreadSince()) {
          expect(
            since,
            inInclusiveRange(
              secondFrom - relayCrossDeviceSkewSeconds,
              secondTo - relayCrossDeviceSkewSeconds,
            ),
          );
        }
      },
    );

    test('a channel whose fallback failed catches up over its whole window '
        'again', () async {
      int now() => DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final session =
          _FakeRelaySession(
              memberships: [_membership(_channelA, myPk)],
              metadata: [_meta(id: _channelA, name: 'general')],
              recentMessages: [message('old', now() - 3600, mention: true)],
            )
            // The batch fails and so does its websocket fallback, without a
            // deadline: the catch-up completes with no events for the channel.
            ..messageBatchError = Exception('batch unavailable')
            ..messageHistoryError = Exception('CLOSED: socket error');
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      final notifier = container.read(channelsProvider.notifier);
      await container.read(channelsProvider.future);
      await _waitUntil(() => session.queryBatches.any(isUnread));
      await _settle();
      expect(notifier.observedUnreadEventsByChannel[_channelA], isNull);

      session
        ..messageBatchError = null
        ..messageHistoryError = null;
      final before = session.queryBatches.where(isUnread).length;
      await notifier.refresh();
      await _waitUntil(
        () => session.queryBatches.where(isUnread).length > before,
      );
      await _settle();
      expect(
        [
          for (final filter in session.queryBatches.lastWhere(isUnread))
            filter.since,
        ],
        [0, 0],
      );
      expect(notifier.observedUnreadEventsByChannel[_channelA]?.keys, ['old']);
    });

    test('a channel unmuted since catches up over its whole window', () async {
      int now() => DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
        recentMessages: [message('backlog', now() - 3600)],
      );
      final mutes = _TestMutes(initiallyMuted: {_channelA});
      final container = _buildContainer(session: session, mutes: mutes);
      addTearDown(container.dispose);
      final notifier = container.read(channelsProvider.notifier);
      Future<void> refreshed() async {
        final before = session.queryBatches.where(isUnread).length;
        await notifier.refresh();
        await _waitUntil(
          () => session.queryBatches.where(isUnread).length > before,
        );
        await _settle();
      }

      await container.read(channelsProvider.future);
      await _waitUntil(() => session.queryBatches.any(isUnread));
      await _settle();
      // Muted: ordinary backlog doesn't count, and the next catch-up is
      // incremental.
      await refreshed();
      expect(notifier.observedUnreadEventsByChannel[_channelA], isNull);
      expect(
        session.queryBatches.lastWhere(isUnread).map((f) => f.since),
        everyElement(isNot(0)),
      );

      mutes.unmute();
      await refreshed();
      expect(
        session.queryBatches.lastWhere(isUnread).map((f) => f.since),
        everyElement(0),
      );
      expect(notifier.observedUnreadEventsByChannel[_channelA]?.keys, [
        'backlog',
      ]);
    });

    test('a rebuilt channel list catches up over the whole window', () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
        recentMessages: [message('seen', 10000, mention: true)],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await _waitUntil(() => session.queryBatches.any(isUnread));
      await _settle();

      container.invalidate(channelsProvider);
      final before = session.queryBatches.where(isUnread).length;
      await container.read(channelsProvider.future);
      await _waitUntil(
        () => session.queryBatches.where(isUnread).length > before,
      );
      expect(
        [
          for (final filter in session.queryBatches.lastWhere(isUnread))
            filter.since,
        ],
        [0, 0],
      );
    });

    test('quiet agent messages never count as unread', () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
        recentMessages: [
          message('quiet-mention', 30, mention: true, quiet: true),
          message('mention', 20, mention: true),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await _waitUntil(() => session.queryBatches.any(isUnread));
      await _settle();
      session.emit(message('quiet-live', 60, quiet: true));
      session.emit(message('live', 50));
      await _settle();
      final notifier = container.read(channelsProvider.notifier);
      expect(
        notifier.observedUnreadEventsByChannel[_channelA]?.keys,
        unorderedEquals(['mention', 'live']),
      );
      expect(notifier.latestObservedByChannel[_channelA], 50);
    });

    // Quiet messages that fill the bounded catch-up window can hide an older
    // unread event, so the full window still counts its oldest one, as plain
    // backlog even when it mentions the reader.
    for (final (count, deleted, expected) in [
      (999, false, <String>[]),
      (1000, false, ['quiet-0']),
      (999, true, <String>[]),
      (1000, true, ['quiet-1']),
    ]) {
      test(
        '$count quiet messages, deletion $deleted, record $expected',
        () async {
          final session = _FakeRelaySession(
            memberships: [_membership(_channelA, myPk)],
            metadata: [_meta(id: _channelA, name: 'general')],
            recentMessages: [
              for (var i = count - 1; i >= 0; i--)
                message('quiet-$i', 100 + i, mention: true, quiet: true),
              if (deleted)
                const NostrEvent(
                  id: 'quiet-deletion',
                  pubkey: 'alice',
                  createdAt: 2000,
                  kind: EventKind.deletion,
                  tags: [
                    ['h', _channelA],
                    ['e', 'quiet-0'],
                  ],
                  content: '',
                  sig: 'sig',
                ),
            ],
          );
          final container = _buildContainer(session: session);
          addTearDown(container.dispose);
          await container.read(channelsProvider.future);
          await _waitUntil(() => session.queryBatches.any(isUnread));
          await _settle();
          final notifier = container.read(channelsProvider.notifier);
          expect(
            notifier.observedUnreadEventsByChannel[_channelA]?.keys ?? const [],
            expected,
          );
          for (final event
              in notifier.observedUnreadEventsByChannel[_channelA]?.values ??
                  const <ObservedUnreadEvent>[]) {
            expect(event.highPriority, isFalse);
            expect(event.channelCatchUp, isTrue);
          }
          if (expected.isEmpty) return;

          // Deleting the marker reopens the whole window, which still hides
          // an older unread event, so the next quiet message stands in. The
          // deletion lands while an incremental catch-up is out, which must
          // not restore the cursor the deletion cleared.
          session.pauseNextUnreadCatchUpQuery();
          final parked = notifier.refresh();
          await session.nextUnreadCatchUpQueryStarted;
          session.emit(
            NostrEvent(
              id: 'marker-deletion',
              pubkey: 'alice',
              createdAt: 3000,
              kind: EventKind.deletion,
              tags: [
                const ['h', _channelA],
                ['e', expected.single],
              ],
              content: '',
              sig: 'sig',
            ),
          );
          await _settle();
          session.resumePausedUnreadCatchUpQuery();
          await parked;
          await _settle();
          final before = session.queryBatches.where(isUnread).length;
          await notifier.refresh();
          await _waitUntil(
            () => session.queryBatches.where(isUnread).length > before,
          );
          await _settle();
          expect(
            session.queryBatches.lastWhere(isUnread).map((f) => f.since),
            everyElement(0),
          );
          expect(notifier.observedUnreadEventsByChannel[_channelA]?.keys, [
            deleted ? 'quiet-2' : 'quiet-1',
          ]);
        },
      );
    }

    // HTTP fails ordinarily, then a per-filter websocket fallback times out:
    // the batch is unavailable and no later chunk is sent.
    test('a websocket fallback deadline makes the batch unavailable', () async {
      final ids = [for (var i = 0; i < 6; i++) 'chunk-channel-$i'];
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          for (final id in ids) _membership(id, myPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          for (final id in ids) _meta(id: id, name: id),
        ],
        recentMessages: [message('m1', 30)],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);
      await container.read(channelsProvider.future);
      await _settle();
      general() => container
          .read(channelsProvider)
          .value!
          .firstWhere((channel) => channel.id == _channelA);
      final lastMessageAt = general().lastMessageAt;
      expect(lastMessageAt, isNotNull);
      session
        ..messageBatchError = Exception('bridge down')
        ..messageHistoryError = deadline();
      await container.read(channelsProvider.notifier).refresh();
      await _settle();
      // Unavailable, not empty: known timestamps survive.
      expect(general().lastMessageAt, lastMessageAt);
      // Each batch stops after its first chunk of four filters.
      expect(session.messageHistoryCount % 4, 0);
      expect(session.messageHistoryCount, lessThan(2 * ids.length));
    });
  });

  test(
    'loads all channel timestamps through one batched relay query',
    () async {
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          _membership(_channelB, myPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(id: _channelB, name: 'direct', channelType: 'dm'),
        ],
        recentMessages: const [
          NostrEvent(
            id: 'stream-message',
            pubkey: 'alice',
            createdAt: 30,
            kind: EventKind.streamMessageV2,
            tags: [
              ['h', _channelA],
            ],
            content: 'hello',
            sig: 'sig',
          ),
          NostrEvent(
            id: 'dm-message',
            pubkey: 'alice',
            createdAt: 40,
            kind: 9,
            tags: [
              ['h', _channelB],
            ],
            content: 'hello privately',
            sig: 'sig',
          ),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      final channels = await container.read(channelsProvider.future);

      await _waitUntil(() => session.queryBatches.length == 2);
      expect(session.queryBatches, hasLength(2));
      expect(session.queryBatches.first, hasLength(2));
      expect(
        session.queryBatches.first
            .map((filter) => filter.tags['#h']!.single)
            .toSet(),
        {_channelA, _channelB},
      );
      expect(session.queryBatches.last, hasLength(4));
      for (final channelId in [_channelA, _channelB]) {
        expect(
          session.queryBatches.last
              .where((filter) => filter.tags['#h']!.single == channelId)
              .map((filter) => filter.kinds),
          [
            EventKind.channelMessageEventKinds,
            [EventKind.deletion, EventKind.nip29DeleteEvent],
          ],
        );
      }
      expect(
        session.queryBatches.last.every(
          (filter) => filter.limit == 1000 && filter.since == 0,
        ),
        isTrue,
      );
      expect(
        session.historyFilters.where((filter) {
          final kinds = filter.kinds.toSet();
          return kinds.length == EventKind.channelMessageEventKinds.length &&
              kinds.containsAll(EventKind.channelMessageEventKinds);
        }),
        isEmpty,
      );
      expect(
        channels.firstWhere((channel) => channel.id == _channelA).lastMessageAt,
        DateTime.fromMillisecondsSinceEpoch(30 * 1000, isUtc: true),
      );
      expect(
        channels.firstWhere((channel) => channel.id == _channelB).lastMessageAt,
        DateTime.fromMillisecondsSinceEpoch(40 * 1000, isUtc: true),
      );
    },
  );

  test('ephemeral (TTL) channels appear in the list', () async {
    // Regression: previously the provider unconditionally dropped any channel
    // with a `ttl` tag, which made TTL channels invisible on iOS even when the
    // user was a member. They should be included so the existing
    // `_EphemeralBadge` UI in `channels_page.dart` can render them.
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk), _membership(_channelB, myPk)],
      metadata: [
        _meta(id: _channelA, name: 'general'),
        _meta(
          id: _channelB,
          name: 'agent-creation-deep-dive',
          ttlSeconds: 86400,
        ),
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    final channels = await container.read(channelsProvider.future);

    expect(
      channels.map((c) => c.name),
      containsAll(['general', 'agent-creation-deep-dive']),
    );
    final ephemeral = channels.firstWhere(
      (c) => c.name == 'agent-creation-deep-dive',
    );
    expect(ephemeral.isEphemeral, isTrue);
    expect(ephemeral.ttlSeconds, 86400);
  });

  test(
    'Huddle-linked one-hour private streams stay out of channel lists',
    () async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          _membership(_channelB, myPk, ownerPubkey: myPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(
            id: _channelB,
            name: 'huddle-22222222',
            ttlSeconds: 3600,
            visibility: 'private',
          ),
        ],
        huddleStarts: [
          NostrEvent(
            id: 'huddle-start',
            pubkey: myPk,
            createdAt: now,
            kind: EventKind.huddleStarted,
            tags: const [
              ['h', _channelA],
            ],
            content: '{"ephemeral_channel_id":"$_channelB"}',
            sig: 'sig',
          ),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      final channels = await container.read(channelsProvider.future);

      expect(channels.map((channel) => channel.id), [_channelA]);
    },
  );

  test(
    'Huddle-linked private streams stay hidden when relay overrides the TTL',
    () async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          _membership(_channelB, myPk, ownerPubkey: myPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(
            id: _channelB,
            name: 'huddle-22222222',
            ttlSeconds: 60,
            visibility: 'private',
          ),
        ],
        huddleStarts: [
          NostrEvent(
            id: 'huddle-start',
            pubkey: myPk,
            createdAt: now,
            kind: EventKind.huddleStarted,
            tags: const [
              ['h', _channelA],
            ],
            content: '{"ephemeral_channel_id":"$_channelB"}',
            sig: 'sig',
          ),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      final channels = await container.read(channelsProvider.future);

      expect(channels.map((channel) => channel.id), [_channelA]);
    },
  );

  test(
    'forged Huddle links do not hide unrelated one-hour private streams',
    () async {
      final now = DateTime.now().millisecondsSinceEpoch ~/ 1000;
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          _membership(_channelB, myPk, ownerPubkey: 'actual-owner'),
        ],
        metadata: [
          _meta(id: _channelA, name: 'general'),
          _meta(
            id: _channelB,
            name: 'private-hour-stream',
            ttlSeconds: 3600,
            visibility: 'private',
          ),
        ],
        huddleStarts: [
          NostrEvent(
            id: 'forged-huddle-start',
            pubkey: myPk,
            createdAt: now,
            kind: EventKind.huddleStarted,
            tags: const [
              ['h', _channelA],
            ],
            content: '{"ephemeral_channel_id":"$_channelB"}',
            sig: 'sig',
          ),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      final channels = await container.read(channelsProvider.future);

      expect(channels.map((channel) => channel.id), contains(_channelB));
    },
  );

  test('hidden DMs are filtered from the channel list', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk), _membership(_channelB, myPk)],
      metadata: [
        _meta(id: _channelA, name: 'Alice', channelType: 'dm'),
        _meta(id: _channelB, name: 'Bob', channelType: 'dm'),
      ],
      hiddenDmEvents: [
        _hiddenDms([_channelA], pubkey: myPk),
      ],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    final channels = await container.read(channelsProvider.future);

    expect(channels.map((c) => c.id), [_channelB]);
    expect(
      session.historyFilters.any(
        (filter) =>
            filter.kinds.contains(EventKind.dmVisibility) &&
            filter.tags['#p']?.single == myPk,
      ),
      isTrue,
    );
  });

  test('a DM opened ahead of the list is not found after it is left', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'Alice', channelType: 'dm')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);
    final notifier = container.read(channelsProvider.notifier);
    final dm = (await container.read(channelsProvider.future)).single;

    // A fast open finds the DM from its own metadata before the list has it.
    session.memberships = [];
    await notifier.refresh();
    notifier.rememberOpened(dm);
    expect(notifier.channelById(_channelA)?.id, _channelA);

    // Once listed, the list owns it, so leaving drops it everywhere.
    session.memberships = [_membership(_channelA, myPk)];
    await notifier.refresh();
    session.memberships = [];
    await notifier.refresh();
    expect(container.read(channelsProvider).value, isEmpty);
    expect(notifier.channelById(_channelA), isNull);
    expect(container.read(channelByIdProvider(_channelA)), isNull);

    // The open's background refresh can list the DM before the open
    // remembers it. The list still owns it.
    session.memberships = [_membership(_channelA, myPk)];
    await notifier.refresh();
    notifier.rememberOpened(dm);
    session.memberships = [];
    await notifier.refresh();
    expect(notifier.channelById(_channelA), isNull);
  });

  test('a DM opened ahead of the list stays in its community', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'Alice', channelType: 'dm')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);
    final notifier = container.read(channelsProvider.notifier);
    final dm = (await container.read(channelsProvider.future)).single;
    session.memberships = [];
    await notifier.refresh();
    notifier.rememberOpened(dm);
    final found = <String?>[];
    final lookup = container.listen(
      channelByIdProvider(_channelA),
      (_, next) => found.add(next?.id),
      fireImmediately: true,
    );
    addTearDown(lookup.close);
    expect(found, [_channelA]);

    // Neither community lists the DM, yet the open lookup lets it go.
    container
        .read(relayConfigProvider.notifier)
        .update(baseUrl: 'https://community-b.example');
    await container.read(channelsProvider.future);
    await _settle();
    expect(lookup.read(), isNull);
    expect(
      container.read(channelsProvider.notifier).channelById(_channelA),
      isNull,
    );

    // Likewise across an identity switch.
    notifier.rememberOpened(dm);
    expect(lookup.read()?.id, _channelA);
    container.read(_testPubkeyProvider.notifier).set('other');
    await container.read(channelsProvider.future);
    await _settle();
    expect(lookup.read(), isNull);
  });

  test(
    'archived and unlisted kind:39000 flags reach Channel (covers TTL auto-archive)',
    () async {
      // The relay's TTL reaper auto-archives expired ephemeral channels and
      // republishes kind:39000 with `["archived", "true"]`. The Channel needs
      // `archivedAt != null` so the `_SliverChannelsList` filter
      // (`!channel.isArchived`) hides it from the sidebar after expiry.
      // Previously the mobile parser ignored the `archived` tag, so expired
      // TTL channels would have stayed visible after the `!isEphemeral` guard
      // was removed.
      final session = _FakeRelaySession(
        memberships: [
          _membership(_channelA, myPk),
          _membership(_channelB, myPk),
        ],
        metadata: [
          _meta(id: _channelA, name: 'active', unlisted: true),
          _meta(
            id: _channelB,
            name: 'expired-ttl',
            ttlSeconds: 86400,
            archived: true,
          ),
        ],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      final channels = await container.read(channelsProvider.future);
      final expired = channels.firstWhere((c) => c.name == 'expired-ttl');
      expect(expired.isArchived, isTrue);
      expect(expired.isEphemeral, isTrue);
      // The active channel must not be flagged archived.
      final active = channels.firstWhere((c) => c.name == 'active');
      expect(active.isArchived, isFalse);
      // The bare `["unlisted"]` flag: an unlisted namesake does not hold a
      // name in the relay's listed-name rule (starter setup relies on it).
      expect(active.isUnlisted, isTrue);
      expect(expired.isUnlisted, isFalse);
    },
  );

  test(
    'archive transition invalidates cached channelDetailsProvider',
    () async {
      // Codex review v2 caught: if a TTL channel is opened (caching its
      // ChannelDetails) and then the reaper archives it, the cached details
      // — built from the pre-archive kind:39000 — would clobber the newer
      // archivedAt set on the base Channel during `mergeDetails`. We invalidate
      // the details provider when the archived state flips so the next
      // mergeDetails sees fresh data.
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'active')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      // Initial load.
      final initial = await container.read(channelsProvider.future);
      expect(initial.single.isArchived, isFalse);

      // Prime the detail cache.
      final detailsFiltersBefore = session.historyFilters
          .where((f) => f.kinds.contains(39000) && f.tags['#d'] != null)
          .length;
      await container.read(channelDetailsProvider(_channelA).future);
      final detailsFetchesAfterPrime =
          session.historyFilters
              .where((f) => f.kinds.contains(39000) && f.tags['#d'] != null)
              .length -
          detailsFiltersBefore;
      expect(detailsFetchesAfterPrime, 1);

      // Simulate the reaper auto-archiving the channel by swapping the
      // metadata the fake returns, then refreshing the channels provider.
      session.metadata
        ..clear()
        ..add(_meta(id: _channelA, name: 'active', archived: true));
      await container.read(channelsProvider.notifier).refresh();
      final refreshed = container.read(channelsProvider).value!;
      expect(refreshed.single.isArchived, isTrue);

      // Take a fresh baseline AFTER the refresh — the refresh itself issues a
      // `kinds:[39000], #d:[id]` query as part of channel metadata refetch and
      // we must not count that toward our invalidation assertion. Only the
      // fetch triggered by the second `channelDetailsProvider` read should be
      // attributed to invalidation.
      final detailsFiltersAfterRefresh = session.historyFilters
          .where((f) => f.kinds.contains(39000) && f.tags['#d'] != null)
          .length;

      // Reading the details provider again must trigger a fresh fetch — proving
      // the prior cache was invalidated by the archive transition. Without
      // invalidation, Riverpod would return the cached pre-archive details and
      // no new `kinds:[39000], #d:[id]` filter would be sent.
      await container.read(channelDetailsProvider(_channelA).future);
      final detailsFetchesFromInvalidation =
          session.historyFilters
              .where((f) => f.kinds.contains(39000) && f.tags['#d'] != null)
              .length -
          detailsFiltersAfterRefresh;
      expect(detailsFetchesFromInvalidation, greaterThan(0));
    },
  );

  test(
    'keeps cached channels and live subscriptions during reconnect',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      final initial = await container.read(channelsProvider.future);
      expect(initial.single.name, 'general');
      await _waitUntil(() => session.subscribeFilters.length == 1);
      expect(session.subscribeFilters, hasLength(1));

      session.setStatus(SessionStatus.reconnecting);
      final reconnecting = await container.read(channelsProvider.future);

      expect(reconnecting.single.name, 'general');
      expect(session.subscribeFilters, hasLength(1));
      expect(session.unsubscribeCount, 0);
    },
  );

  test(
    'waits for destination channels after a disconnected community switch',
    () async {
      final session = _FakeRelaySession(
        memberships: [_membership(_channelA, myPk)],
        metadata: [_meta(id: _channelA, name: 'general')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(
        (await container.read(channelsProvider.future)).single.name,
        'general',
      );

      session.setStatus(SessionStatus.disconnected);
      session.memberships = [_membership(_channelB, myPk)];
      session.metadata = [_meta(id: _channelB, name: 'random')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://new-community.example');
      await Future<void>.delayed(Duration.zero);
      expect(container.read(channelsProvider).isLoading, isTrue);
      expect(container.read(channelsProvider.notifier).hasLoaded, isFalse);
      var destinationReady = false;
      final destination = container.read(channelsProvider.future).then((value) {
        destinationReady = true;
        return value;
      });
      await _settle();
      expect(destinationReady, isFalse);

      session.setStatus(SessionStatus.connected);
      await Future<void>.delayed(Duration.zero);

      expect((await destination).single.name, 'random');
      expect(container.read(channelsProvider).isLoading, isFalse);
    },
  );

  test('recovers an initial fetch failure after reconnecting', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'general')],
      membershipFailures: 1,
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await expectLater(container.read(channelsProvider.future), throwsException);

    session.setStatus(SessionStatus.reconnecting);
    session.setStatus(SessionStatus.connected);
    await Future<void>.delayed(Duration.zero);

    final recovered = await container.read(channelsProvider.future);
    expect(recovered.single.name, 'general');
  });

  test(
    'preserves a successfully loaded empty list while disconnected',
    () async {
      final session = _FakeRelaySession(memberships: [], metadata: []);
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);
      final fetchCount = session.historyFilters.length;

      session.setStatus(SessionStatus.reconnecting);
      expect(await container.read(channelsProvider.future), isEmpty);
      expect(session.historyFilters, hasLength(fetchCount));
    },
  );

  test('initial fetch issues membership + metadata queries', () async {
    final session = _FakeRelaySession(
      memberships: [_membership(_channelA, myPk)],
      metadata: [_meta(id: _channelA, name: 'general')],
    );
    final container = _buildContainer(session: session);
    addTearDown(container.dispose);

    await container.read(channelsProvider.future);

    expect(session.membershipQueryFilters, isNotEmpty);
    expect(session.membershipQueryFilters.first.kinds, [39002]);
    expect(session.membershipQueryFilters.first.tags['#p'], [myPk]);
    expect(
      session.historyFilters.any(
        (filter) =>
            filter.kinds.contains(39000) &&
            filter.tags['#d']?.contains(_channelA) == true,
      ),
      isTrue,
    );

    // And one live subscription on the resulting channel.
    await _waitUntil(() => session.subscribeFilters.length == 1);
    expect(session.subscribeFilters, hasLength(1));
  });

  // A reply also sent to the channel shows in the timeline, but it is still a
  // thread reply: it dots only when it mentions the reader or sits in one of
  // their threads. Elsewhere it bolds the channel like a top-level post.
  group('replies also sent to the channel', () {
    final otherRoot = 'a' * 64;
    final ownRoot = 'b' * 64;
    bool isUnread(List<NostrFilter> batch) =>
        batch.isNotEmpty && batch.every((filter) => filter.since != null);
    NostrEvent post(
      String id,
      int createdAt, {
      String author = 'alice',
      String? parentId,
      bool broadcast = false,
      bool mention = false,
    }) => NostrEvent(
      id: id,
      pubkey: author,
      createdAt: createdAt,
      kind: EventKind.streamMessageV2,
      tags: [
        const ['h', _channelA],
        if (parentId != null) ['e', parentId, '', 'reply'],
        if (broadcast) const ['broadcast', '1'],
        if (mention) const ['p', myPk],
      ],
      content: 'hi',
      sig: 'sig',
    );

    for (final (name, setup, follow, mention, dotted) in [
      ("someone else's thread", <NostrEvent>[], false, false, false),
      (
        "someone else's thread, mentioning the reader",
        <NostrEvent>[],
        false,
        true,
        true,
      ),
      ('a thread the reader follows', <NostrEvent>[], true, false, true),
      (
        'a thread the reader started',
        [post(ownRoot, 10, author: myPk)],
        false,
        false,
        true,
      ),
      (
        'a thread the reader replied in',
        [post('own-reply', 11, author: myPk, parentId: otherRoot)],
        false,
        false,
        true,
      ),
    ]) {
      test('$name ${dotted ? 'dots' : 'only bolds'} the channel', () async {
        final root = setup.any((event) => event.id == ownRoot)
            ? ownRoot
            : otherRoot;
        final session = _FakeRelaySession(
          memberships: [_membership(_channelA, myPk)],
          metadata: [_meta(id: _channelA, name: 'general')],
        );
        final follows = _TestFollows();
        final container = _buildContainer(session: session, follows: follows);
        addTearDown(container.dispose);
        await container.read(channelsProvider.future);
        await _waitUntil(() => session.queryBatches.any(isUnread));
        await _settle();
        if (follow) {
          follows.choose(followed: {root}, channelByRoot: {root: _channelA});
        }
        setup.forEach(session.emit);
        final broadcast = post(
          'broadcast',
          20,
          parentId: root,
          broadcast: true,
          mention: mention,
        );
        session.emit(broadcast);
        await _settle();

        final notifier = container.read(channelsProvider.notifier);
        final channels = container.read(channelsProvider).value!;
        final observed =
            notifier.observedUnreadEventsByChannel[_channelA]!['broadcast']!;
        expect(observed.highPriority, dotted);
        expect(observed.attention, dotted);
        // Activity lists the thread only when the reply earns attention.
        expect(
          notifier.observedThreadReplies(channels),
          dotted ? [broadcast] : isEmpty,
        );
        // The app icon counts it as attention only when it dots. Otherwise
        // it is ordinary unread, the same as a new top-level post.
        final badge = container.read(unreadBadgeProvider);
        expect(badge.highPriorityCount, dotted ? 1 : 0);
        expect(badge.generalUnreadCount, dotted ? 0 : 1);

        void expectHighlights(
          Map<String, int> contexts, {
          required bool unread,
          required bool attention,
        }) {
          final highlights = computeChannelListHighlights(
            channels: channels,
            readState: ReadStateState(
              isReady: true,
              pubkey: myPk,
              contexts: contexts,
              version: 0,
            ),
            channelsNotifier: notifier,
          );
          expect(highlights.unread, unread ? {_channelA} : isEmpty);
          expect(highlights.attention, attention ? {_channelA} : isEmpty);
        }

        expectHighlights(const {}, unread: !dotted, attention: dotted);
        // Read by its own mark either way, as desktop reads it: reaching the
        // bottom of the channel does not read it.
        expect(observed.channelCatchUp, isFalse);
        expectHighlights(
          {activityContextKey(_channelA): 100},
          unread: !dotted,
          attention: dotted,
        );
        expectHighlights(
          {msgContextKey('broadcast'): 20},
          unread: false,
          attention: false,
        );
      });
    }
  });
}

const _channelA = '11111111-1111-4111-8111-111111111111';
const _channelB = '22222222-2222-4222-8222-222222222222';
const _channelD = '44444444-4444-4444-8444-444444444444';
const _otherPk = 'someone-else';

String _generatedChannelId(int index) =>
    '${index.toString().padLeft(8, '0')}-0000-4000-8000-000000000000';

/// Build a kind:39002 membership event tagged with the channel id and member.
NostrEvent _membership(
  String channelId,
  String pubkey, {
  String? additionalPubkey,
  String? ownerPubkey,
  int createdAt = 1,
  String? eventId,
}) => NostrEvent(
  id: eventId ?? 'mem-$channelId',
  pubkey: 'creator',
  createdAt: createdAt,
  kind: 39002,
  tags: [
    ['d', channelId],
    if (ownerPubkey != null) ['p', ownerPubkey, '', 'owner'],
    if (ownerPubkey == null || ownerPubkey != pubkey) ['p', pubkey],
    if (additionalPubkey != null) ['p', additionalPubkey],
  ],
  content: '',
  sig: 'sig',
);

NostrEvent _hiddenDms(List<String> channelIds, {required String pubkey}) =>
    NostrEvent(
      id: 'hidden-${channelIds.join('-')}',
      pubkey: 'relay',
      createdAt: 2,
      kind: EventKind.dmVisibility,
      tags: [
        ['d', pubkey],
        ['p', pubkey],
        for (final channelId in channelIds) ['h', channelId],
      ],
      content: '',
      sig: 'sig',
    );

/// Build a kind:39000 channel metadata event.
NostrEvent _meta({
  required String id,
  required String name,
  String channelType = 'stream',
  String visibility = 'open',
  int createdAt = 1,
  int? ttlSeconds,
  String? creator,
  String? description,
  bool archived = false,
  bool unlisted = false,
  List<String> participants = const [],
}) => NostrEvent(
  id: 'meta-$id',
  pubkey: 'creator',
  createdAt: createdAt,
  kind: 39000,
  tags: [
    ['d', id],
    ['name', name],
    if (creator != null) ['P', creator],
    if (description != null) ['about', description],
    ['t', channelType],
    [visibility == 'private' ? 'private' : 'public'],
    if (ttlSeconds != null) ['ttl', '$ttlSeconds'],
    if (archived) ['archived', 'true'],
    if (unlisted) ['unlisted'],
    for (final participant in participants) ['p', participant],
  ],
  content: '',
  sig: 'sig',
);

ProviderContainer _buildContainer({
  required _FakeRelaySession session,
  _TestMutes? mutes,
  _TestFollows? follows,
  List<Override> overrides = const [],
}) {
  return ProviderContainer(
    retry: (_, _) => null,
    overrides: [
      ...overrides,
      if (mutes != null) channelMutesProvider.overrideWith(() => mutes),
      if (follows != null) threadFollowsProvider.overrideWith(() => follows),
      appLifecycleProvider.overrideWith(() => _FakeAppLifecycleNotifier()),
      relaySessionProvider.overrideWith(() => session),
      // Route the pubkey through a mutable notifier so tests can switch the
      // signing identity mid-flight the way an account change does at runtime.
      myPubkeyProvider.overrideWith((ref) => ref.watch(_testPubkeyProvider)),
    ],
  );
}

/// Channel mutes a test sets directly.
class _TestMutes extends ChannelMutesNotifier {
  _TestMutes({this.initiallyMuted = const {}});

  final Set<String> initiallyMuted;

  @override
  ChannelMutesState build() => _muted(initiallyMuted);

  void unmute() => state = _muted(const {});

  static ChannelMutesState _muted(Set<String> channelIds) => ChannelMutesState(
    isReady: true,
    store: ChannelMuteStore(
      channels: {
        for (final id in channelIds)
          id: const ChannelMuteEntry(muted: true, updatedAt: 1),
      },
    ),
  );
}

/// Mutable stand-in for the signing identity derived from the active community.
class _TestPubkeyNotifier extends Notifier<String?> {
  @override
  String? build() => 'me';

  void set(String? pubkey) => state = pubkey;
}

final _testPubkeyProvider = NotifierProvider<_TestPubkeyNotifier, String?>(
  _TestPubkeyNotifier.new,
);

// Active filter activity counts only signed messages.
const _activityKey =
    '0000000000000000000000000000000000000000000000000000000000000001';
const _aliceKey =
    '0000000000000000000000000000000000000000000000000000000000000002';
final _activityPk = derivePublicKeyHex(_activityKey);

class _ActivityPubkey extends _TestPubkeyNotifier {
  @override
  String? build() => _activityPk;
}

NostrEvent _signedPost(
  int at, {
  String channel = _channelA,
  String content = 'hi',
  String key = _activityKey,
  List<List<String>> tags = const [],
}) => NostrEvent.fromJson(
  signNostrEvent(
    kind: EventKind.streamMessageV2,
    content: content,
    secretKey: key,
    createdAt: at,
    tags: [
      ['h', channel],
      ...tags,
    ],
  ).toMap(),
);

bool _isGapBatch(List<NostrFilter> batch) =>
    batch.any((filter) => filter.until != null && filter.limit == 1);

/// Drains pending microtasks so provider rebuilds and awaited writes land.
Future<void> _settle() async {
  for (var i = 0; i < 20; i++) {
    await Future<void>.delayed(Duration.zero);
  }
}

/// Waits in real time for work a real isolate finishes, such as catch-up
/// activity, which lands after unread readiness.
Future<void> _waitForIsolate(bool Function() predicate) async {
  final deadline = DateTime.now().add(const Duration(seconds: 5));
  while (!predicate() && DateTime.now().isBefore(deadline)) {
    await Future<void>.delayed(const Duration(milliseconds: 10));
  }
}

Future<void> _waitUntil(bool Function() predicate) async {
  for (var i = 0; i < 100; i++) {
    if (predicate()) return;
    await Future<void>.delayed(Duration.zero);
  }
  fail('Timed out waiting for asynchronous provider work');
}

/// Fake [RelaySessionNotifier] that returns canned query results and records
/// subscriptions.
int _explicitChannelValues(List<NostrFilter> filters) => filters.fold(
  0,
  (total, filter) => total + (filter.tags['#h']?.length ?? 0),
);

class _FakeRelaySession extends RelaySessionNotifier {
  _FakeRelaySession({
    required this.memberships,
    this.membershipPages,
    this.repeatLastMembershipPage = false,
    this.maxMembershipPageRequests,
    this.metadata = const [],
    this.metadataPages,
    this.metadataPageBuilder,
    this.repeatLastMetadataPage = false,
    this.maxMetadataPageRequests,
    this.hiddenDmEvents = const [],
    this.huddleStarts = const [],
    this.creations = const [],
    this.recentMessages = const [],
    this.deletionChannels = const {},
    this.enforceHistoryLimits = false,
    this.membershipFailures = 0,
  });

  List<NostrEvent> memberships;
  final List<List<NostrEvent>>? membershipPages;
  final bool repeatLastMembershipPage;
  final int? maxMembershipPageRequests;
  List<NostrEvent> metadata;
  final List<List<NostrEvent>>? metadataPages;
  final List<NostrEvent> Function(int pageIndex)? metadataPageBuilder;
  final bool repeatLastMetadataPage;
  final int? maxMetadataPageRequests;
  final List<NostrEvent> hiddenDmEvents;
  final List<NostrEvent> huddleStarts;
  final List<NostrEvent> creations;
  Completer<void>? creationGate;
  int creationReads = 0;
  bool failCreations = false;
  List<NostrEvent> recentMessages;
  final Map<String, String> deletionChannels;
  final bool enforceHistoryLimits;
  int membershipFailures;
  int directoryFailures = 0;
  bool failClaimedMemberMetadataQuery = false;
  bool failClaimedUnreadCatchUpQuery = false;
  int membershipRequestCount = 0;
  int metadataPageRequestCount = 0;

  final List<NostrFilter> historyFilters = [];

  /// While set, member metadata reads wait for it to complete.
  Completer<void>? metadataGate;
  // Holds the next latest-message batch, which a list refresh awaits after it
  // has merged the directory.
  Completer<void>? latestBatchGate;
  final List<List<NostrFilter>> queryBatches = [];
  Object? messageBatchError;
  // Fails batches made only of the catch-up's activity gap reads.
  Object? gapBatchError;
  // Fails a batch that only reads deletions of given posts.
  Object? headDeletionBatchError;
  // Holds the next batch of activity gap reads.
  Completer<void>? gapBatchGate;
  Object? latestBatchError;
  Object? messageHistoryError;
  int messageHistoryCount = 0;

  final List<NostrFilter> directoryQueryFilters = [];
  final List<NostrFilter> membershipQueryFilters = [];
  final List<NostrFilter> subscribeFilters = [];
  // The channel list notice subscription, kept apart from the live channel
  // subscriptions these tests count.
  final List<NostrFilter> listNoticeFilters = [];
  final Map<int, void Function(NostrEvent)> _listNoticeListeners = {};
  final Map<
    int,
    (NostrFilter, void Function(NostrEvent), void Function(String message)?)
  >
  _subscriptions = {};
  int _nextSubscriptionKey = 0;
  Completer<void>? _pausedSubscribe;
  Completer<void>? _subscribeStarted;
  Completer<void>? _pausedDirectory;
  bool _pauseClaimed = false;
  Completer<void>? _directoryStarted;
  Completer<void>? _pausedHiddenDm;
  Completer<void>? _hiddenDmStarted;
  Completer<void>? _claimedHiddenDm;
  Completer<void>? _pausedMemberMetadata;
  Completer<void>? _memberMetadataStarted;
  Completer<void>? _claimedMemberMetadata;
  Completer<void>? _pausedHuddleStarts;
  Completer<void>? _huddleStartsStarted;
  Completer<void>? _claimedHuddleStarts;
  Completer<void>? _pausedUnreadCatchUp;
  Completer<void>? _unreadCatchUpStarted;
  Completer<void>? _claimedUnreadCatchUp;
  int unsubscribeCount = 0;
  int totalSubscribeCount = 0;
  int subscribeFailures = 0;
  int successfulSubscribesBeforeFailure = 0;

  Set<String> get activeChannels => {
    for (final (filter, _, _) in _subscriptions.values)
      ...filter.tags['#h'] ?? const <String>[],
  };

  int get activeSubscriptionCount => _subscriptions.length;

  Future<void> get nextSubscribeStarted async {
    final started = _subscribeStarted;
    if (started == null) {
      throw StateError('No paused subscription is pending');
    }
    await started.future;
  }

  void pauseNextSubscribe() {
    if (_pausedSubscribe != null) {
      throw StateError('A subscription is already paused');
    }
    _pausedSubscribe = Completer<void>();
    _subscribeStarted = Completer<void>();
  }

  void resumePausedSubscribe() {
    final paused = _pausedSubscribe;
    if (paused == null) throw StateError('No subscription is paused');
    paused.complete();
  }

  /// Holds the next directory query open so a community or identity switch can
  /// be interleaved between the request and its response.
  void pauseNextDirectoryQuery() {
    if (_pausedDirectory != null) {
      throw StateError('A directory query is already paused');
    }
    _pausedDirectory = Completer<void>();
    _directoryStarted = Completer<void>();
  }

  /// Completes once the paused directory query has been requested.
  Future<void> get nextDirectoryQueryStarted async {
    final started = _directoryStarted;
    if (started == null) {
      throw StateError('No directory query is pending');
    }
    await started.future;
  }

  /// Releases the paused directory query so its response lands.
  void resumePausedDirectoryQuery() {
    final paused = _pausedDirectory;
    if (paused == null) throw StateError('No directory query is paused');
    paused.complete();
  }

  /// Holds the next hidden-DM query open, one shot only.
  ///
  /// One shot matters: the refresh that follows the switch issues its own
  /// hidden-DM query, and it must be able to finish while the earlier scope's
  /// query is still parked. That is the window Jed's second probe describes.
  void pauseNextHiddenDmQuery() {
    if (_pausedHiddenDm != null) {
      throw StateError('A hidden-DM query is already paused');
    }
    _pausedHiddenDm = Completer<void>();
    _hiddenDmStarted = Completer<void>();
  }

  /// Completes once the parked hidden-DM query has been requested.
  Future<void> get nextHiddenDmQueryStarted async {
    final started = _hiddenDmStarted;
    if (started == null) {
      throw StateError('No hidden-DM query is pending');
    }
    await started.future;
  }

  /// Releases the parked hidden-DM query so its response lands.
  void resumePausedHiddenDmQuery() {
    final paused = _claimedHiddenDm ?? _pausedHiddenDm;
    if (paused == null) throw StateError('No hidden-DM query is paused');
    paused.complete();
  }

  /// Holds the next Huddle-start query open, one shot only.
  ///
  /// Parks the older refresh AFTER membership discovery has landed, so the
  /// only guard left between the park and the member-snapshot write is the
  /// fence on this leg.
  void pauseNextHuddleStartQuery() {
    if (_pausedHuddleStarts != null) {
      throw StateError('A Huddle-start query is already paused');
    }
    _pausedHuddleStarts = Completer<void>();
    _huddleStartsStarted = Completer<void>();
  }

  /// Completes once the parked Huddle-start query has been requested.
  Future<void> get nextHuddleStartQueryStarted async {
    final started = _huddleStartsStarted;
    if (started == null) {
      throw StateError('No Huddle-start query is pending');
    }
    await started.future;
  }

  /// Releases the parked Huddle-start query so its response lands.
  void resumePausedHuddleStartQuery() {
    final paused = _claimedHuddleStarts ?? _pausedHuddleStarts;
    if (paused == null) throw StateError('No Huddle-start query is paused');
    _claimedHuddleStarts = null;
    _pausedHuddleStarts = null;
    paused.complete();
  }

  /// Holds the next member-metadata query open, one shot only.
  void pauseNextMemberMetadataQuery() {
    if (_pausedMemberMetadata != null) {
      throw StateError('A member-metadata query is already paused');
    }
    _pausedMemberMetadata = Completer<void>();
    _memberMetadataStarted = Completer<void>();
  }

  /// Completes once the parked member-metadata query has been requested.
  Future<void> get nextMemberMetadataQueryStarted async {
    final started = _memberMetadataStarted;
    if (started == null) {
      throw StateError('No member-metadata query is pending');
    }
    await started.future;
  }

  /// Releases the parked member-metadata query so its response lands.
  void resumePausedMemberMetadataQuery() {
    final paused = _claimedMemberMetadata ?? _pausedMemberMetadata;
    if (paused == null) throw StateError('No member-metadata query is paused');
    paused.complete();
  }

  /// Holds the next unread catch-up batch open, one shot only.
  ///
  /// The catch-up runs detached from the refresh that starts it, so this is the
  /// window where a community or identity switch can land between the request
  /// and the writes its response drives.
  void pauseNextUnreadCatchUpQuery() {
    if (_pausedUnreadCatchUp != null) {
      throw StateError('An unread catch-up query is already paused');
    }
    _pausedUnreadCatchUp = Completer<void>();
    _unreadCatchUpStarted = Completer<void>();
  }

  /// Completes once the parked unread catch-up batch has been requested.
  Future<void> get nextUnreadCatchUpQueryStarted async {
    final started = _unreadCatchUpStarted;
    if (started == null) {
      throw StateError('No unread catch-up query is pending');
    }
    await started.future;
  }

  /// Releases the parked unread catch-up batch so its response lands.
  void resumePausedUnreadCatchUpQuery() {
    final paused = _claimedUnreadCatchUp ?? _pausedUnreadCatchUp;
    if (paused == null) {
      throw StateError('No unread catch-up query is paused');
    }
    paused.complete();
  }

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    historyFilters.add(filter);
    if ((filter.tags['#h']?.length ?? 0) > kMaxExplicitChannelValues) {
      throw Exception('restricted: too many explicit channels');
    }
    if (filter.kinds.contains(39002) && filter.tags['#d'] != null) {
      throw StateError('Membership rosters must reuse discovery events');
    }
    if (filter.kinds.contains(39002) && filter.tags['#p'] != null) {
      membershipRequestCount++;
      if (membershipFailures > 0) {
        membershipFailures--;
        throw Exception('membership fetch failed');
      }
      // Membership query — return all memberships we have for this pubkey.
      final myPk = filter.tags['#p']?.single;
      return memberships
          .where(
            (e) =>
                e.tags.any((t) => t.length >= 2 && t[0] == 'p' && t[1] == myPk),
          )
          .toList();
    }
    if (filter.kinds.contains(EventKind.dmVisibility)) {
      // Claim the parked slot so the switch's own refresh runs unblocked.
      final paused = _pausedHiddenDm;
      if (paused != null) {
        _claimedHiddenDm = paused;
        _pausedHiddenDm = null;
        _hiddenDmStarted!.complete();
        _hiddenDmStarted = null;
        await paused.future;
      }
      return hiddenDmEvents;
    }
    if (filter.kinds.contains(9007)) {
      creationReads++;
      await creationGate?.future;
      if (failCreations) throw Exception('receipt unavailable');
      final ids = filter.tags['#h'] ?? const <String>[];
      return creations.where((e) => ids.contains(e.getTagValue('h'))).toList();
    }
    if (filter.kinds.contains(EventKind.huddleStarted)) {
      // Claim the parked slot so the newer refresh's own Huddle query runs
      // unblocked: one shot, exactly like the hidden-DM and member-metadata hooks.
      final paused = _pausedHuddleStarts;
      if (paused != null) {
        _claimedHuddleStarts = paused;
        _pausedHuddleStarts = null;
        _huddleStartsStarted!.complete();
        _huddleStartsStarted = null;
        await paused.future;
      }
      final parents = (filter.tags['#h'] ?? const <String>[]).toSet();
      return huddleStarts
          .where((event) => parents.contains(event.getTagValue('h')))
          .toList();
    }
    if (filter.kinds.contains(39000)) {
      final ids = filter.tags['#d']?.toSet();
      if (ids == null) {
        throw StateError('Directory queries must use the HTTP query bridge');
      }
      final snapshot = metadata
          .where((e) => ids.contains(e.getTagValue('d')))
          .toList();
      final paused = _pausedMemberMetadata;
      if (paused != null) {
        _claimedMemberMetadata = paused;
        _pausedMemberMetadata = null;
        _memberMetadataStarted!.complete();
        _memberMetadataStarted = null;
        await paused.future;
        if (failClaimedMemberMetadataQuery) {
          throw Exception('member-metadata fetch failed');
        }
      }
      await metadataGate?.future;
      return snapshot;
    }
    // Per-filter websocket fallback of a message batch.
    messageHistoryCount++;
    if (messageHistoryError case final error?) throw error;
    return const [];
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (filters case [final filter]
        when filter.kinds.length == 1 &&
            filter.kinds.single == 39002 &&
            filter.tags['#p'] != null) {
      membershipQueryFilters.add(filter);
      if (membershipFailures > 0) {
        membershipFailures--;
        throw Exception('membership fetch failed');
      }
      final requestIndex = membershipRequestCount++;
      final maxRequests = maxMembershipPageRequests;
      if (maxRequests != null && requestIndex >= maxRequests) {
        throw StateError('Unexpected membership page request');
      }
      final pages = membershipPages;
      if (pages != null) {
        if (requestIndex < pages.length) return List.of(pages[requestIndex]);
        if (repeatLastMembershipPage && pages.isNotEmpty) {
          return List.of(pages.last);
        }
        return const [];
      }
      if (filter.until != null) return const [];
      final myPk = filter.tags['#p']?.single;
      return memberships
          .where(
            (event) => event.tags.any(
              (tag) => tag.length >= 2 && tag[0] == 'p' && tag[1] == myPk,
            ),
          )
          .toList();
    }
    if (filters case [final filter]
        when filter.kinds.length == 1 &&
            filter.kinds.single == 39000 &&
            !filter.tags.containsKey('#d')) {
      directoryQueryFilters.add(filter);
      // Snapshot the directory contents at request time so a paused response
      // reflects the community that issued it, not whichever community is
      // active when the response is released.
      final directorySnapshot = List.of(metadata);
      final paused = _pausedDirectory;
      // Only the first query after the pause waits; a later one meanwhile
      // (a new scope's load) answers at once.
      if (paused != null && !_pauseClaimed) {
        _pauseClaimed = true;
        _directoryStarted!.complete();
        await paused.future;
        _pausedDirectory = null;
        _directoryStarted = null;
        _pauseClaimed = false;
      }
      if (directoryFailures > 0) {
        directoryFailures--;
        throw Exception('directory fetch failed');
      }
      final requestIndex = metadataPageRequestCount++;
      final maxRequests = maxMetadataPageRequests;
      if (maxRequests != null && requestIndex >= maxRequests) {
        throw StateError('Unexpected directory page request');
      }
      final pageBuilder = metadataPageBuilder;
      if (pageBuilder != null) return List.of(pageBuilder(requestIndex));
      final pages = metadataPages;
      if (pages != null) {
        if (requestIndex < pages.length) return List.of(pages[requestIndex]);
        if (repeatLastMetadataPage && pages.isNotEmpty) {
          return List.of(pages.last);
        }
        return const [];
      }
      if (filter.until != null) return const [];
      final since = filter.since;
      return [
        for (final event in directorySnapshot)
          if (since == null || event.createdAt >= since) event,
      ];
    }
    queryBatches.add(filters);
    if (_explicitChannelValues(filters) > kMaxExplicitChannelValues) {
      throw RelayException(400, '{"error":"too many explicit channels"}');
    }
    if (messageBatchError case final error?) throw error;
    if (headDeletionBatchError case final error?
        when filters.isNotEmpty &&
            filters.every(
              (filter) =>
                  filter.tags.containsKey('#e') &&
                  filter.kinds.contains(EventKind.deletion),
            )) {
      throw error;
    }
    if (_isGapBatch(filters)) {
      if (gapBatchError case final error?) throw error;
      final gate = gapBatchGate;
      gapBatchGate = null;
      await gate?.future;
    }
    // The unread catch-up is the only batch that carries `since` on every
    // filter; the latest-message batch leaves it null. Snapshot the messages at
    // request time so a parked response reflects the scope that asked for it.
    final isUnreadCatchUp =
        filters.isNotEmpty && filters.every((filter) => filter.since != null);
    if (!isUnreadCatchUp && latestBatchError != null) throw latestBatchError!;
    if (!isUnreadCatchUp) {
      final gate = latestBatchGate;
      latestBatchGate = null;
      await gate?.future;
    }
    final messageSnapshot = List.of(recentMessages);
    if (isUnreadCatchUp) {
      // Claim the parked slot so the refresh that follows the switch can run
      // its own catch-up unblocked while this one stays parked.
      final paused = _pausedUnreadCatchUp;
      if (paused != null) {
        _claimedUnreadCatchUp = paused;
        _pausedUnreadCatchUp = null;
        _unreadCatchUpStarted!.complete();
        _unreadCatchUpStarted = null;
        await paused.future;
        if (failClaimedUnreadCatchUpQuery) {
          throw Exception('unread catch-up fetch failed');
        }
      }
    }
    final matched = <String, NostrEvent>{};
    for (final filter in filters) {
      final matches = messageSnapshot.where((event) {
        if (!filter.kinds.contains(event.kind)) return false;
        if (filter.since case final since? when event.createdAt < since) {
          return false;
        }
        if (filter.until case final until? when event.createdAt > until) {
          return false;
        }
        for (final entry in filter.tags.entries) {
          final tagName = entry.key.startsWith('#')
              ? entry.key.substring(1)
              : entry.key;
          // Relay #h matching uses stored channel metadata for h-less NIP-09.
          if (tagName == 'h' &&
              event.kind == EventKind.deletion &&
              event.channelId == null &&
              entry.value.contains(deletionChannels[event.id])) {
            continue;
          }
          if (!event.tags.any(
            (tag) =>
                tag.length > 1 &&
                tag[0] == tagName &&
                entry.value.contains(tag[1]),
          )) {
            return false;
          }
        }
        return true;
      }).toList();
      if (enforceHistoryLimits) {
        matches.sort((a, b) => b.createdAt.compareTo(a.createdAt));
      }
      for (final event
          in enforceHistoryLimits ? matches.take(filter.limit) : matches) {
        matched[event.id] = event;
      }
    }
    return matched.values.toList();
  }

  @override
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
    required void Function(RelaySubscriptionStatus) onStatusChanged,
    bool resumeSince = true,
  }) => subscribe(filter, onEvent, onClosed: onClosed);

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
    if (filter.kinds.contains(EventKind.memberAddedNotification)) {
      final key = ++_nextSubscriptionKey;
      listNoticeFilters.add(filter);
      _listNoticeListeners[key] = onEvent;
      return () {
        if (_listNoticeListeners.remove(key) != null) {
          listNoticeFilters.remove(filter);
        }
      };
    }
    totalSubscribeCount++;
    subscribeFilters.add(filter);
    final paused = _pausedSubscribe;
    if (paused != null) {
      _subscribeStarted!.complete();
      await paused.future;
      _pausedSubscribe = null;
      _subscribeStarted = null;
    }
    if (subscribeFailures > 0 && successfulSubscribesBeforeFailure == 0) {
      subscribeFailures--;
      subscribeFilters.remove(filter);
      throw StateError('live subscription failed');
    }
    if (successfulSubscribesBeforeFailure > 0) {
      successfulSubscribesBeforeFailure--;
    }
    final subscriptionKey = ++_nextSubscriptionKey;
    _subscriptions[subscriptionKey] = (filter, onEvent, onClosed);
    return () {
      final subscription = _subscriptions.remove(subscriptionKey);
      if (subscription == null) return;
      unsubscribeCount++;
      subscribeFilters.remove(subscription.$1);
    };
  }

  void closeSubscriptionContaining(String channelId, String message) {
    final entry = _subscriptions.entries.singleWhere(
      (entry) => entry.value.$1.tags['#h']?.contains(channelId) ?? false,
    );
    _subscriptions.remove(entry.key);
    subscribeFilters.remove(entry.value.$1);
    entry.value.$3?.call(message);
  }

  void setStatus(SessionStatus status) {
    state = SessionState(status: status);
  }

  /// Emit a live event to all subscribers.
  void emit(NostrEvent event) {
    for (final (_, listener, _) in List.of(_subscriptions.values)) {
      listener(event);
    }
    for (final listener in List.of(_listNoticeListeners.values)) {
      listener(event);
    }
  }
}

NostrEvent _huddleStart({
  required String pubkey,
  required String parent,
  required String backing,
}) => NostrEvent(
  id: 'huddle-start-$parent',
  pubkey: pubkey,
  createdAt: DateTime.now().millisecondsSinceEpoch ~/ 1000,
  kind: EventKind.huddleStarted,
  tags: [
    ['h', parent],
  ],
  content: '{"ephemeral_channel_id":"$backing"}',
  sig: 'sig',
);

/// Fails every HTTP channel-history request the way an unavailable query
/// bridge does, and records how many relay requests each batch keeps open.
class _BridgeDownRelaySession extends _FakeRelaySession {
  _BridgeDownRelaySession({
    required super.memberships,
    required super.metadata,
  });

  final inFlight = <String, int>{};
  final peakInFlight = <String, int>{};

  static String? _batchOf(List<NostrFilter> filters) {
    final filter = filters.firstOrNull;
    if (filter == null) return null;
    if (filter.kinds.contains(EventKind.huddleStarted)) return 'huddle';
    if (!filter.kinds.contains(EventKind.streamMessageV2)) return null;
    return filter.since == null ? 'latest' : 'unread';
  }

  Future<T> _held<T>(String? batch, Future<T> Function() request) async {
    if (batch == null) return request();
    final open = inFlight[batch] = (inFlight[batch] ?? 0) + 1;
    peakInFlight[batch] = max(peakInFlight[batch] ?? 0, open);
    try {
      // Hold each request so everything the batch has started overlaps.
      await Future<void>.delayed(const Duration(milliseconds: 1));
      return await request();
    } finally {
      inFlight[batch] = inFlight[batch]! - 1;
    }
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) {
    final batch = _batchOf(filters);
    return _held(
      batch,
      () => batch == null
          ? super.queryRelay(filters, timeout: timeout)
          : throw Exception('query bridge unavailable'),
    );
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) => _held(
    _batchOf([filter]),
    () => super.fetchHistory(filter, timeout: timeout),
  );
}

class _ActivityReadState extends ReadStateNotifier {
  bool unavailable = false;

  @override
  ReadStateState build() {
    if (unavailable) throw StateError('read state unavailable');
    return const ReadStateState.inert();
  }

  void markReadThrough(String channelId, int at) =>
      state = state.copyWithContext(channelId, at);
}

class _FakeAppLifecycleNotifier extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}
