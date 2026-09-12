import 'package:buzz/features/channels/channel_detail_page.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/theme/theme_provider.dart';
import 'package:shared_preferences/shared_preferences.dart';
import 'package:flutter/material.dart';
import '../../features/channels/channel_detail_page_test.dart'
    show admissionDmHarness;

import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../community/community_storage_test.dart' show FakeSecureStorage;
import 'owner_search_generation_test.dart'
    show AdmissionCommunities, AdmissionAuth, drainAdmission;
import '../crypto/nip_oa_test.dart' show authTag, profile;

void main() {
  for (final huddle in [false, true]) {
    for (final retired in [false, true]) {
      testWidgets('R1 ${huddle ? 'huddle' : 'DM'} admission retired=$retired', (
        tester,
      ) async {
        final owner = nostr.Keys.generate();
        final agent = nostr.Keys.generate();
        final owned = profile(agent, [authTag(owner, agent.public)]);
        final initial = Community.create(
          name: 'A',
          relayUrl: 'https://a.example',
          nsec: owner.nsec,
        );
        final next = Community.create(
          name: 'B',
          relayUrl: 'https://b.example',
          nsec: owner.nsec,
        );
        final storage = CommunityStorage(secure: FakeSecureStorage());
        await storage.saveActiveId(initial.id);
        final list = AdmissionCommunities(initial);
        final session = RelaySessionNotifier();
        final socket = _RecordingSocket();
        SharedPreferences.setMockInitialValues({});
        final prefs = await SharedPreferences.getInstance();
        final container = ProviderContainer.test(
          overrides: [
            savedPrefsProvider.overrideWithValue(prefs),
            authProvider.overrideWith(AdmissionAuth.new),
            communityStorageProvider.overrideWithValue(storage),
            communityListProvider.overrideWith(() => list),
            relaySessionProvider.overrideWith(() => session),
            channelMembersProvider('admission').overrideWith(
              (ref) async => [
                ChannelMember(
                  pubkey: agent.public,
                  role: 'member',
                  joinedAt: DateTime(2025),
                ),
              ],
            ),
          ],
        );
        await container.read(authProvider.future);
        await container.read(activeCommunityProvider.future);
        container.read(relaySessionProvider);
        await session.debugHandleConnected();
        session.debugAttachSocketForTest(socket);
        container.listen(agentOwnersProvider, (_, _) {});
        await drainAdmission();
        // Empty directory is a legitimate ready source, not a fake owner map.
        session.debugHandleMessage(['EOSE', socket.history.single]);
        await drainAdmission();
        expect(await container.read(agentOwnersProvider.future), isEmpty);
        if (huddle) {
          await container.read(channelMembersProvider('admission').future);
          container.listen(debugHuddleProfileUpdates('admission'), (_, _) {});
        } else {
          await tester.pumpWidget(
            UncontrolledProviderScope(
              container: container,
              child: await admissionDmHarness([owner.public, agent.public]),
            ),
          );
        }
        await drainAdmission();
        final id = socket.profileIds(huddle).single;
        expect(id, startsWith('l-'));
        socket.ready(session);
        await drainAdmission();
        session.debugHandleMessage(['EVENT', id, owned.toJson()]);
        if (retired) {
          await storage.saveActiveId(next.id);
          list.replace(next);
          await container.read(activeCommunityProvider.future);
        }
        session.debugFlushEventBuffer();
        await drainAdmission();
        final observed = container
            .read(userCacheProvider.notifier)
            .profileOwners;
        // Keep the real consumer and widget alive across lazy invalidation.
        if (retired) {
          container.read(relaySessionProvider);
          if (huddle) container.read(debugHuddleProfileUpdates('admission'));
          await tester.pump();
          await session.debugHandleConnected();
          session.debugAttachSocketForTest(socket);
          if (huddle) container.read(debugHuddleProfileUpdates('admission'));
          container.read(agentOwnersProvider);
          await drainAdmission();
          socket.ready(session);
          await drainAdmission();
        }
        await tester.pump();
        final authority = await container.read(agentOwnersProvider.future);
        final currentDm = socket.profileIds(huddle).last;
        if (retired) expect(currentDm, isNot(id));
        socket.ready(session);
        await drainAdmission();
        final nextOwner = nostr.Keys.generate();
        final advancing = profile(agent, [
          authTag(nextOwner, agent.public),
        ], createdAt: 101);
        session.debugHandleMessage(['EVENT', currentDm, advancing.toJson()]);
        session.debugFlushEventBuffer();
        await tester.pump();
        final recovery = await container.read(agentOwnersProvider.future);
        await tester.pumpWidget(const SizedBox.shrink());
        container.dispose();
        await tester.pump(const Duration(milliseconds: 600));
        expect(recovery, {agent.public: nextOwner.public});
        expect(authority, retired ? isEmpty : {agent.public: owner.public});
        expect(observed, retired ? isEmpty : {agent.public: owner.public});
      });
    }
  }
}

class _RecordingSocket extends RelaySocket {
  _RecordingSocket()
    : super(
        wsUrl: 'wss://unused.example',
        nsec: null,
        onMessage: (_) {},
        onConnected: () {},
        onDisconnected: (_) {},
      );
  final requests = <List<dynamic>>[];
  Iterable<String> get history => requests
      .where((r) => (r[1] as String).startsWith('h-'))
      .map((r) => r[1] as String)
      .toList();
  Iterable<String> profileIds(bool huddle) => requests
      .where(
        (r) =>
            (r[2]['kinds'] as List).contains(0) &&
            (huddle
                ? r[2]['limit'] == 0
                : (r[2]['kinds'] as List).contains(10100)),
      )
      .map((r) => r[1] as String);
  void ready(RelaySessionNotifier session) {
    for (final r in requests.toList()) {
      session.debugHandleMessage(['EOSE', r[1]]);
    }
  }

  @override
  void send(List<dynamic> payload) {
    if (payload.first == 'REQ') requests.add(payload);
  }
}
