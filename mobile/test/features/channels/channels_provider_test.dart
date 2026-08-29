import 'dart:async';

import 'package:fake_async/fake_async.dart';
import 'package:flutter/widgets.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/relay/relay.dart';

part 'channels_provider_live_cases.dart';
part 'channels_provider_terminal_cases.dart';

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
  const myPk = 'me';

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
    'ordinary refresh settles a superseded directory load for retry',
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

      // A foreground, reconnect, pull-to-refresh, or membership update can
      // start an ordinary refresh while Browse is still loading discovery.
      await container.read(channelsProvider.notifier).refresh();

      session.resumePausedDirectoryQuery();
      await directory;
      await _settle();

      expect(
        container.read(channelDirectoryLoadStatusProvider).status,
        ChannelDirectoryLoadStatus.error,
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
    'community switch after directory success discards a late failure',
    () async {
      final session = _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'community-a-open')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);

      // The directory query succeeds, then the refresh parks on the member-count
      // query and fails there after the switch. A retired failure must not
      // overwrite the new scope's list or push it into an error state.
      session.memberships = [_membership(_channelA, myPk)];
      session.pauseNextMemberCountQuery();
      final staleRefresh = container
          .read(channelsProvider.notifier)
          .retryDirectory();
      await session.nextMemberCountQueryStarted;

      session.memberships = [_membership(_channelB, myPk)];
      session.metadata = [_meta(id: _channelB, name: 'community-b-general')];
      container
          .read(relayConfigProvider.notifier)
          .update(baseUrl: 'https://community-b.example');
      await container.read(channelsProvider.future);

      session.failClaimedMemberCountQuery = true;
      session.resumePausedMemberCountQuery();
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
    'identity switch after directory success discards the late refresh',
    () async {
      final session = _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'first-identity-open')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);

      // Directory success, then the refresh parks on the hidden-DM query while
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
    'identity switch after directory success discards a late failure',
    () async {
      final session = _FakeRelaySession(
        memberships: const [],
        metadata: [_meta(id: _channelA, name: 'first-identity-open')],
      );
      final container = _buildContainer(session: session);
      addTearDown(container.dispose);

      expect(await container.read(channelsProvider.future), isEmpty);

      session.memberships = [_membership(_channelA, myPk)];
      session.pauseNextMemberCountQuery();
      final staleRefresh = container
          .read(channelsProvider.notifier)
          .retryDirectory();
      await session.nextMemberCountQueryStarted;

      session.memberships = [_membership(_channelB, _otherPk)];
      session.metadata = [_meta(id: _channelB, name: 'second-identity-joined')];
      container.read(_testPubkeyProvider.notifier).set(_otherPk);
      await container.read(channelsProvider.future);

      session.failClaimedMemberCountQuery = true;
      session.resumePausedMemberCountQuery();
      await staleRefresh;
      await _settle();

      final current = container.read(channelsProvider);
      expect(current.hasError, isFalse);
      final ids = current.requireValue.map((channel) => channel.id);
      expect(ids, isNot(contains(_channelA)));
      expect(ids, contains(_channelB));
    },
  );

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

    // Park an older refresh on its Huddle-start query. Both of its membership
    // fetches have already landed, so channel A is the list it is carrying.
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
      expect(session.queryBatches.last, hasLength(2));
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

  test(
    'archived kind:39000 metadata sets Channel.isArchived (covers TTL auto-archive)',
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
          _meta(id: _channelA, name: 'active'),
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
    'refreshes cached channels after a disconnected community switch',
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
      expect(container.read(channelsProvider).value?.single.name, 'general');

      session.setStatus(SessionStatus.connected);
      await Future<void>.delayed(Duration.zero);

      expect(container.read(channelsProvider).value?.single.name, 'random');
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
}) => NostrEvent(
  id: 'mem-$channelId',
  pubkey: 'creator',
  createdAt: 1,
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
  bool archived = false,
}) => NostrEvent(
  id: 'meta-$id',
  pubkey: 'creator',
  createdAt: createdAt,
  kind: 39000,
  tags: [
    ['d', id],
    ['name', name],
    ['t', channelType],
    [visibility == 'private' ? 'private' : 'public'],
    if (ttlSeconds != null) ['ttl', '$ttlSeconds'],
    if (archived) ['archived', 'true'],
  ],
  content: '',
  sig: 'sig',
);

ProviderContainer _buildContainer({required _FakeRelaySession session}) {
  return ProviderContainer(
    retry: (_, _) => null,
    overrides: [
      appLifecycleProvider.overrideWith(() => _FakeAppLifecycleNotifier()),
      relaySessionProvider.overrideWith(() => session),
      // Route the pubkey through a mutable notifier so tests can switch the
      // signing identity mid-flight the way an account change does at runtime.
      myPubkeyProvider.overrideWith((ref) => ref.watch(_testPubkeyProvider)),
    ],
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

/// Drains pending microtasks so provider rebuilds and awaited writes land.
Future<void> _settle() async {
  for (var i = 0; i < 20; i++) {
    await Future<void>.delayed(Duration.zero);
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
    this.recentMessages = const [],
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
  List<NostrEvent> recentMessages;
  int membershipFailures;
  int directoryFailures = 0;
  bool failClaimedMemberCountQuery = false;
  bool failClaimedUnreadCatchUpQuery = false;
  int membershipRequestCount = 0;
  int metadataPageRequestCount = 0;

  final List<NostrFilter> historyFilters = [];
  final List<List<NostrFilter>> queryBatches = [];
  final List<NostrFilter> directoryQueryFilters = [];
  final List<NostrFilter> membershipQueryFilters = [];
  final List<NostrFilter> subscribeFilters = [];
  final Map<
    int,
    (NostrFilter, void Function(NostrEvent), void Function(String message)?)
  >
  _subscriptions = {};
  int _nextSubscriptionKey = 0;
  Completer<void>? _pausedSubscribe;
  Completer<void>? _subscribeStarted;
  Completer<void>? _pausedDirectory;
  Completer<void>? _directoryStarted;
  Completer<void>? _pausedHiddenDm;
  Completer<void>? _hiddenDmStarted;
  Completer<void>? _claimedHiddenDm;
  Completer<void>? _pausedMemberCount;
  Completer<void>? _memberCountStarted;
  Completer<void>? _claimedMemberCount;
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
  /// Parks the older refresh AFTER both membership fetches have landed, so the
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

  /// Holds the next member-count query open, one shot only.
  void pauseNextMemberCountQuery() {
    if (_pausedMemberCount != null) {
      throw StateError('A member-count query is already paused');
    }
    _pausedMemberCount = Completer<void>();
    _memberCountStarted = Completer<void>();
  }

  /// Completes once the parked member-count query has been requested.
  Future<void> get nextMemberCountQueryStarted async {
    final started = _memberCountStarted;
    if (started == null) {
      throw StateError('No member-count query is pending');
    }
    await started.future;
  }

  /// Releases the parked member-count query so its response lands.
  void resumePausedMemberCountQuery() {
    final paused = _claimedMemberCount ?? _pausedMemberCount;
    if (paused == null) throw StateError('No member-count query is paused');
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
    if (filter.kinds.contains(39002) && filter.tags['#d'] != null) {
      final paused = _pausedMemberCount;
      if (paused != null) {
        _claimedMemberCount = paused;
        _pausedMemberCount = null;
        _memberCountStarted!.complete();
        _memberCountStarted = null;
        await paused.future;
        if (failClaimedMemberCountQuery) {
          throw Exception('member-count fetch failed');
        }
      }
      final ids = (filter.tags['#d'] ?? const <String>[]).toSet();
      return memberships
          .where((event) => ids.contains(event.getTagValue('d')))
          .toList();
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
    if (filter.kinds.contains(EventKind.huddleStarted)) {
      // Claim the parked slot so the newer refresh's own Huddle query runs
      // unblocked: one shot, exactly like the hidden-DM and member-count hooks.
      final paused = _pausedHuddleStarts;
      if (paused != null) {
        _claimedHuddleStarts = paused;
        _pausedHuddleStarts = null;
        _huddleStartsStarted!.complete();
        _huddleStartsStarted = null;
        await paused.future;
      }
      return huddleStarts;
    }
    if (filter.kinds.contains(39000)) {
      final ids = filter.tags['#d']?.toSet();
      if (ids == null) {
        throw StateError('Directory queries must use the HTTP query bridge');
      }
      // Member metadata query — return only matching `d` tags.
      return metadata.where((e) => ids.contains(e.getTagValue('d'))).toList();
    }
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
      if (paused != null) {
        _directoryStarted!.complete();
        await paused.future;
        _pausedDirectory = null;
        _directoryStarted = null;
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
      return filter.until == null ? directorySnapshot : const [];
    }
    queryBatches.add(filters);
    // The unread catch-up is the only batch that carries `since` on every
    // filter; the latest-message batch leaves it null. Snapshot the messages at
    // request time so a parked response reflects the scope that asked for it.
    final isUnreadCatchUp =
        filters.isNotEmpty && filters.every((filter) => filter.since != null);
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
    return messageSnapshot.where((event) {
      return filters.any((filter) {
        if (!filter.kinds.contains(event.kind)) return false;
        for (final entry in filter.tags.entries) {
          final tagName = entry.key.startsWith('#')
              ? entry.key.substring(1)
              : entry.key;
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
      });
    }).toList();
  }

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async {
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
  }
}

class _FakeAppLifecycleNotifier extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}
