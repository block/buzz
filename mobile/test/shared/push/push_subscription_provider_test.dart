import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_mutes/channel_mutes_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/push/push_bridge.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/push/push_relay_capability_provider.dart';
import 'package:buzz/shared/push/push_subscription_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  const activeID = '123e4567-e89b-42d3-a456-426614174000';
  const archivedID = '123e4567-e89b-42d3-a456-426614174001';
  const nonMemberID = '123e4567-e89b-42d3-a456-426614174002';

  test(
    'derives desired subscriptions from nsec, membership, and mute state',
    () {
      final nsec = nostr.Keys.generate().nsec;
      final subscriptions = desiredBuzzPushSubscriptions(
        descriptor: _descriptor(),
        community: Community.create(
          name: 'Team',
          relayUrl: 'https://relay.example.com',
          nsec: nsec,
        ),
        channels: [
          channel(activeID),
          channel(archivedID, archived: true),
          channel(nonMemberID, isMember: false),
        ],
        mutedChannelIds: const [activeID, archivedID],
      )?.subscriptions;

      expect(subscriptions, isNotNull);
      expect(subscriptions, hasLength(1));
      expect(subscriptions!.single.filter.pTags, hasLength(1));
      expect(subscriptions.single.ignore, hasLength(2));
      expect(subscriptions.single.ignore.first.kinds, buzzPushRenderableKinds);
      expect(subscriptions.single.ignore.last.hTags, [activeID]);
    },
  );

  test('subscribes channel-wide to joined DMs and own Solo chats only', () {
    final keys = nostr.Keys.generate();
    const dmID = '123e4567-e89b-42d3-a456-426614174003';
    const soloID = '123e4567-e89b-42d3-a456-426614174004';
    const archivedSoloID = '123e4567-e89b-42d3-a456-426614174005';
    const otherSoloID = '123e4567-e89b-42d3-a456-426614174006';
    const streamID = '123e4567-e89b-42d3-a456-426614174007';
    const mutedSoloID = '123e4567-e89b-42d3-a456-426614174008';
    const legacySoloID = '123e4567-e89b-42d3-a456-426614174009';
    Channel solo(String id, {String? createdBy, bool archived = false}) =>
        channel(
          id,
          channelType: 'stream',
          visibility: 'private',
          labels: const ['buzz.solo'],
          createdBy: createdBy ?? keys.public,
          archived: archived,
        );
    final subscriptions = desiredBuzzPushSubscriptions(
      descriptor: _descriptor(),
      community: Community.create(
        name: 'Team',
        relayUrl: 'https://relay.example.com',
        nsec: keys.nsec,
      ),
      channels: [
        channel(dmID),
        solo(soloID),
        solo(archivedSoloID, archived: true),
        solo(otherSoloID, createdBy: 'author'),
        channel(streamID, channelType: 'stream', visibility: 'private'),
        solo(mutedSoloID),
        channel(legacySoloID, channelType: 'stream', visibility: 'private'),
      ],
      legacySoloChannelIds: const [legacySoloID],
      mutedChannelIds: const [mutedSoloID],
    )?.subscriptions;

    // A muted Solo chat is left out, like a muted channel, so only a
    // mention pushes it.
    expect(subscriptions, hasLength(2));
    expect(subscriptions!.first.ignore, hasLength(1));
    expect(subscriptions.last.filter.kinds, buzzPushChannelKinds);
    expect(subscriptions.last.filter.hTags, [dmID, soloID, legacySoloID]);
  });

  test('fills the lease channel count with the most recent Solo chats', () {
    final keys = nostr.Keys.generate();
    const dmCount = 10;
    const soloCount = buzzPushMaxChannelWideChannels;
    final channels = [
      for (var n = 0; n < dmCount; n++) channel(_id(n)),
      for (var n = dmCount; n < dmCount + soloCount; n++) _solo(n, keys.public),
    ];

    final desired = desiredBuzzPushSubscriptions(
      descriptor: _descriptor(),
      community: _community(keys),
      channels: channels,
      mutedChannelIds: const [],
    )!;

    final followed = {
      for (final subscription in desired.subscriptions.skip(1))
        ...?subscription.filter.hTags,
    };
    expect(desired.subscriptions, hasLength(buzzPushMaxSubscriptions));
    expect(followed, hasLength(buzzPushMaxChannelWideChannels));
    expect(followed, containsAll([for (var n = 0; n < dmCount; n++) _id(n)]));
    expect(followed, contains(_id(channels.length - 1)));
    expect(followed, isNot(contains(_id(dmCount + dmCount - 1))));
    expect(desired.omittedSoloChats, dmCount);

    // A relay may advertise any endpoint limit; sizing must not allocate it.
    expect(
      () => desiredBuzzPushSubscriptions(
        descriptor: _descriptor(endpointLength: 1 << 50),
        community: _community(keys),
        channels: channels,
        mutedChannelIds: const [],
      ),
      throwsFormatException,
    );
  });

  for (final (limit, descriptor, soloCount) in [
    (
      'plaintext',
      _descriptor(contentLength: 65536, plaintextLength: 16384),
      400,
    ),
    ('content', _descriptor(contentLength: 24000, plaintextLength: 32768), 400),
    ('NIP-44 plaintext', _descriptor(endpointLength: 40000), 700),
  ]) {
    test('keeps the newest Solo chats within the $limit limit', () async {
      final keys = nostr.Keys.generate();
      const dmCount = 10;
      final channels = [
        for (var n = 0; n < dmCount; n++) channel(_id(n)),
        for (var n = dmCount; n < dmCount + soloCount; n++)
          _solo(n, keys.public),
      ];
      final desired = desiredBuzzPushSubscriptions(
        community: _community(keys),
        descriptor: descriptor,
        channels: channels,
        mutedChannelIds: const [],
      )!;

      final kept = {
        for (final subscription in desired.subscriptions.skip(1))
          ...?subscription.filter.hTags,
      };
      final keptSolos = soloCount - desired.omittedSoloChats;
      expect(keptSolos, inInclusiveRange(1, soloCount - 1));
      expect(kept, {
        for (var n = 0; n < dmCount; n++) _id(n),
        for (var n = channels.length - keptSolos; n < channels.length; n++)
          _id(n),
      });
      // The relay accepts any grant up to its endpoint limit.
      final publication = await _publish(
        descriptor,
        keys,
        desired.subscriptions,
      );
      expect(publication.eventId, 'accepted-id');
      // A grant JSON must escape would outgrow the fit, so it never publishes.
      await expectLater(
        _publish(
          descriptor,
          keys,
          desired.subscriptions,
          endpointGrant: '"' * descriptor.maxEndpointLength,
        ),
        throwsA(
          isA<FormatException>().having(
            (error) => error.message,
            'message',
            'Endpoint grant is not a gateway grant',
          ),
        ),
      );
    });
  }

  for (final (limit, descriptor) in [
    ('content', _descriptor(contentLength: 100)),
    ('plaintext', _descriptor(plaintextLength: 100)),
  ]) {
    test(
      'fails setup when the $limit limit cannot fit DMs and mentions',
      () async {
        final keys = nostr.Keys.generate();
        final channels = [channel(_id(0)), _solo(1, keys.public)];

        // Publication rejects even the lease without Solo chats.
        await expectLater(
          _publish(
            descriptor,
            keys,
            buildDesiredBuzzPushSubscriptions(
              myPubkey: keys.public,
              channelIds: [_id(0)],
            ),
          ),
          throwsFormatException,
        );
        expect(
          () => desiredBuzzPushSubscriptions(
            community: _community(keys),
            descriptor: descriptor,
            channels: channels,
            mutedChannelIds: const [],
          ),
          throwsFormatException,
        );
      },
    );
  }

  test('waits for the Solo record but not past its failure', () async {
    const legacySoloID = '123e4567-e89b-42d3-a456-426614174009';
    final community = Community.create(
      name: 'Team',
      relayUrl: 'https://relay.example.com',
      nsec: nostr.Keys.generate().nsec,
    ).copyWith(pushNotificationsEnabled: true);
    final communities = _RecordingCommunityList();
    final soloRecord = StreamController<MeGroups>();
    addTearDown(soloRecord.close);
    final container = ProviderContainer(
      retry: (_, _) => null,
      overrides: [
        authProvider.overrideWith(_Authenticated.new),
        currentRelayPushDescriptorProvider.overrideWith(
          (ref) async => _descriptor(),
        ),
        activeCommunityProvider.overrideWith((ref) async => community),
        communityListProvider.overrideWith(() => communities),
        channelsProvider.overrideWith(
          () => _FixedChannels([
            channel(activeID),
            channel(legacySoloID, channelType: 'stream', visibility: 'private'),
          ]),
        ),
        channelMutesProvider.overrideWith(_ReadyMutes.new),
        meGroupsProvider.overrideWith((ref) => soloRecord.stream),
      ],
    );
    addTearDown(container.dispose);
    container.listen(pushSubscriptionSyncProvider, (_, _) {});
    await container.read(authProvider.future);
    await container.read(currentRelayPushDescriptorProvider.future);
    await container.read(activeCommunityProvider.future);
    await container.read(channelsProvider.future);
    await pumpEventQueue();

    expect(communities.desired, isEmpty);

    soloRecord.addError(StateError('Solo sync failed'));
    await pumpEventQueue();

    expect(communities.desired, hasLength(1));
    expect(communities.desired.last.last.filter.hTags, [activeID]);

    soloRecord.add(const MeGroups(channels: [legacySoloID]));
    await pumpEventQueue();

    expect(communities.desired, hasLength(2));
    expect(communities.desired.last.last.filter.hTags, [
      activeID,
      legacySoloID,
    ]);
  });

  test('returns no desired subscriptions without a signing identity', () {
    final subscriptions = desiredBuzzPushSubscriptions(
      descriptor: _descriptor(),
      community: Community.create(
        name: 'Team',
        relayUrl: 'https://relay.example.com',
      ),
      channels: [channel(activeID)],
      mutedChannelIds: const [],
    );

    expect(subscriptions, isNull);
  });
}

Channel channel(
  String id, {
  bool isMember = true,
  bool archived = false,
  String channelType = 'dm',
  String visibility = 'open',
  List<String> labels = const [],
  String createdBy = 'author',
  DateTime? lastMessageAt,
}) => Channel(
  id: id,
  name: id,
  channelType: channelType,
  visibility: visibility,
  labels: labels,
  description: '',
  createdBy: createdBy,
  createdAt: DateTime(2026),
  memberCount: 1,
  isMember: isMember,
  archivedAt: archived ? DateTime(2026) : null,
  lastMessageAt: lastMessageAt,
);

class _RecordingCommunityList extends CommunityListNotifier {
  final desired = <List<BuzzPushSubscription>>[];

  @override
  Future<List<Community>> build() async => const [];

  @override
  Future<void> updateDesiredPushSubscriptions(
    String id,
    List<BuzzPushSubscription> desired,
  ) async => this.desired.add(desired);
}

class _FixedChannels extends ChannelsNotifier {
  _FixedChannels(this._channels);

  final List<Channel> _channels;

  @override
  Future<List<Channel>> build() async => _channels;
}

class _ReadyMutes extends ChannelMutesNotifier {
  @override
  ChannelMutesState build() => const ChannelMutesState(isReady: true);
}

class _Authenticated extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.authenticated);
}

final _relayKeys = nostr.Keys.generate();

String _id(int n) =>
    '123e4567-e89b-42d3-a456-${n.toRadixString(16).padLeft(12, '0')}';

Channel _solo(int n, String owner) => channel(
  _id(n),
  channelType: 'stream',
  visibility: 'private',
  labels: const ['buzz.solo'],
  createdBy: owner,
  lastMessageAt: DateTime(2026).add(Duration(minutes: n)),
);

Community _community(nostr.Keys keys) => Community.create(
  name: 'Team',
  relayUrl: 'https://relay.example.com',
  nsec: keys.nsec,
);

/// Defaults to no limit beyond NIP-44's own. The relay advertises 65536 and
/// 32768 in NIP-11 (`crates/buzz-relay/src/nip11.rs`).
BuzzPushLeaseDescriptor _descriptor({
  int contentLength = 1 << 22,
  int plaintextLength = 1 << 21,
  int endpointLength = 4096,
}) => BuzzPushLeaseDescriptor(
  origin: 'wss://relay.example',
  executorKeyId: 'key',
  executorPubkey: _relayKeys.public,
  transport: 'apns',
  maxLeaseTtlSeconds: 3600,
  maxContentLength: contentLength,
  maxPlaintextLength: plaintextLength,
  maxEndpointLength: endpointLength,
  maxStringLength: 512,
);

Future<BuzzPushLeasePublication> _publish(
  BuzzPushLeaseDescriptor descriptor,
  nostr.Keys keys,
  List<BuzzPushSubscription> subscriptions, {
  String? endpointGrant,
}) => publishBuzzDevPushLease(
  grant: BuzzPushEndpointGrant(
    relayOrigin: descriptor.origin,
    relayPubkey: descriptor.executorPubkey,
    installationId: 'c' * 32,
    endpointGrant: endpointGrant ?? 'x' * descriptor.maxEndpointLength,
    endpointHash: 'd' * 64,
    endpointEpoch: 1,
    generation: 9007199254740991,
    expiresAt: 1756212000,
  ),
  descriptor: descriptor,
  nsec: keys.nsec,
  memberPubkey: keys.public,
  subscriptions: subscriptions,
  now: () => DateTime.fromMillisecondsSinceEpoch(1752620000 * 1000),
  submit: ({required kind, required content, required tags, createdAt}) async =>
      const NostrEvent(
        id: 'accepted-id',
        pubkey: '',
        createdAt: 0,
        kind: 0,
        tags: [],
        content: '',
        sig: '',
      ),
);
