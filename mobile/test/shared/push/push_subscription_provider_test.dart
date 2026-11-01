import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_mutes/channel_mutes_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/push/dev_push_lease.dart';
import 'package:buzz/shared/push/push_relay_capability_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:buzz/shared/push/push_subscription_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../community/community_storage_test.dart' show FakeSecureStorage;

void main() {
  const activeID = '123e4567-e89b-42d3-a456-426614174000';
  const archivedID = '123e4567-e89b-42d3-a456-426614174001';
  const nonMemberID = '123e4567-e89b-42d3-a456-426614174002';

  test(
    'derives desired subscriptions from nsec, membership, and mute state',
    () {
      final nsec = nostr.Keys.generate().nsec;
      final subscriptions = desiredBuzzPushSubscriptions(
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
      );

      expect(subscriptions, isNotNull);
      expect(subscriptions, hasLength(1));
      expect(subscriptions!.single.filter.pTags, hasLength(1));
      expect(subscriptions.single.ignore, hasLength(2));
      expect(subscriptions.single.ignore.first.kinds, buzzPushRenderableKinds);
      expect(subscriptions.single.ignore.last.hTags, [activeID]);
    },
  );

  test('passes muted authors through to the ignore filters', () {
    final agent = 'b' * 64;
    final subscriptions = desiredBuzzPushSubscriptions(
      community: Community.create(
        name: 'Team',
        relayUrl: 'https://relay.example.com',
        nsec: nostr.Keys.generate().nsec,
      ),
      channels: const [],
      mutedChannelIds: const [],
      mutedAuthorPubkeys: [agent],
    );

    expect(subscriptions!.single.ignore.last.authors, [agent]);
  });

  group('pushSubscriptionSyncProvider', () {
    final me = nostr.Keys.generate();
    final ownedAgent = 'c' * 64;
    final directoryAgent = 'b' * 64;

    Future<List<BuzzPushSubscription>> sync({
      required bool muteAgents,
      SessionStatus status = SessionStatus.connected,
    }) async {
      final storage = CommunityStorage(secure: FakeSecureStorage());
      final community = Community.create(
        name: 'Team',
        relayUrl: 'https://relay.example.com',
        nsec: me.nsec,
      ).copyWith(pushNotificationsEnabled: true, pushMuteAgents: muteAgents);
      await storage.save(community);
      await storage.saveActiveId(community.id);
      final container = ProviderContainer(
        overrides: [
          communityStorageProvider.overrideWithValue(storage),
          communitySnapshotWriterProvider.overrideWithValue((_) async {}),
          authProvider.overrideWith(_Auth.new),
          relaySessionProvider.overrideWith(() => _Session(status)),
          currentRelayPushDescriptorProvider.overrideWith(
            (_) async => const BuzzPushLeaseDescriptor(
              origin: 'wss://relay.example.com',
              executorKeyId: 'key',
              executorPubkey: 'pubkey',
              transport: 'apns',
              maxLeaseTtlSeconds: 3600,
              maxContentLength: 4096,
              maxPlaintextLength: 4096,
              maxEndpointLength: 2048,
              maxStringLength: 512,
            ),
          ),
          channelsProvider.overrideWith(_Channels.new),
          channelMutesProvider.overrideWith(_Mutes.new),
          agentDirectoryProvider.overrideWith(
            (_) async => [AgentDirectoryEntry(pubkey: directoryAgent)],
          ),
          agentOwnersProvider.overrideWith(
            (_) async => {ownedAgent: me.public},
          ),
        ],
      );
      addTearDown(container.dispose);
      // The sync provider reads these synchronously; load them first.
      addTearDown(
        container.listen(currentRelayPushDescriptorProvider, (_, _) {}).close,
      );
      addTearDown(
        container.listen(pushSubscriptionSyncProvider, (_, _) {}).close,
      );
      await container.read(authProvider.future);
      await container.read(activeCommunityProvider.future);
      await container.read(currentRelayPushDescriptorProvider.future);
      await container.read(channelsProvider.future);
      await container.read(pushSubscriptionSyncProvider.future);
      return (await storage.loadAll()).single.pushSubscriptionState.desired;
    }

    test('ignores directory and profile-verified agents when muted', () async {
      final desired = await sync(muteAgents: true);

      expect(desired.single.ignore.last.authors, [directoryAgent, ownedAgent]);
    });

    test('leaves agents alone by default', () async {
      final desired = await sync(muteAgents: false);

      expect(
        desired.single.ignore.where((f) => f.authors != null),
        hasLength(1),
      );
    });

    test(
      'keeps the last lease while the agent directory is unavailable',
      () async {
        final desired = await sync(
          muteAgents: true,
          status: SessionStatus.disconnected,
        );

        expect(desired, isEmpty);
      },
    );
  });

  test('returns no desired subscriptions without a signing identity', () {
    final subscriptions = desiredBuzzPushSubscriptions(
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

Channel channel(String id, {bool isMember = true, bool archived = false}) =>
    Channel(
      id: id,
      name: id,
      channelType: 'dm',
      visibility: 'open',
      description: '',
      createdBy: 'author',
      createdAt: DateTime(2026),
      memberCount: 1,
      isMember: isMember,
      archivedAt: archived ? DateTime(2026) : null,
    );

class _Auth extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.authenticated);
}

class _Session extends RelaySessionNotifier {
  _Session(this.status);
  final SessionStatus status;
  @override
  SessionState build() => SessionState(status: status);
}

class _Channels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => const [];
}

class _Mutes extends ChannelMutesNotifier {
  @override
  ChannelMutesState build() => const ChannelMutesState(isReady: true);
}
