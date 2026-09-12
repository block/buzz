import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/mentions/mention_candidates_provider.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../community/community_storage_test.dart' show FakeSecureStorage;
import '../crypto/nip_oa_test.dart' show authTag, profile;

void main() {
  for (final boundary in ['search', 'refresh', 'preload']) {
    for (final transition
        in boundary == 'search'
            ? ['unchanged', 'community', 'account', 'ABA']
            : ['unchanged', 'community']) {
      test('$boundary rejects retired $transition evidence', () async {
        final owner = nostr.Keys.generate();
        final agent = nostr.Keys.generate();
        final owned = profile(agent, [
          authTag(owner, agent.public),
        ], content: '{"name":"agent"}');
        final initial = Community.create(
          name: 'A',
          relayUrl: 'https://a.example',
          nsec: owner.nsec,
        );
        final next = Community.create(
          name: 'B',
          relayUrl: transition == 'community'
              ? 'https://b.example'
              : initial.relayUrl,
          nsec: nostr.Keys.generate().nsec,
        );
        final storage = CommunityStorage(secure: FakeSecureStorage());
        await storage.saveActiveId(initial.id);
        final list = AdmissionCommunities(initial);
        final entered = Completer<void>();
        final response = Completer<http.Response>();
        final session = RelaySessionNotifier(
          httpClient: MockClient((request) {
            expect(request.url.host, 'a.example');
            entered.complete();
            return response.future;
          }),
        );
        final container = ProviderContainer.test(
          overrides: [
            authProvider.overrideWith(AdmissionAuth.new),
            communityStorageProvider.overrideWithValue(storage),
            communityListProvider.overrideWith(() => list),
            relaySessionProvider.overrideWith(() => session),
            if (boundary == 'search') ...[
              channelsProvider.overrideWith(AdmissionChannels.new),
              channelMembersProvider('probe').overrideWith((ref) async => []),
              agentDirectoryProvider.overrideWith((ref) async => []),
              currentPubkeyProvider.overrideWithValue(owner.public),
            ],
          ],
        );
        await container.read(authProvider.future);
        await container.read(activeCommunityProvider.future);
        final cache = container.read(userCacheProvider.notifier);
        final search = mentionUserSearchProvider('agent');
        Future<bool>? pending;
        if (boundary == 'search') {
          container.listen(search, (_, _) {});
          await entered.future;
        } else {
          pending = boundary == 'refresh'
              ? cache.refresh([agent.public])
              : cache.preload([agent.public]);
          if (boundary == 'preload') {
            await Future<void>.delayed(const Duration(milliseconds: 60));
          }
        }
        // Resolve the real asynchronous active-community provider, leaving its
        // indirect config dependents lazy until the old HTTP acceptance runs.
        for (final community in [
          if (transition != 'unchanged') next,
          if (transition == 'ABA') initial,
        ]) {
          await storage.saveActiveId(community.id);
          list.replace(community);
          await container.read(activeCommunityProvider.future);
        }
        if (boundary == 'search') {
          response.complete(http.Response(jsonEncode([owned.toJson()]), 200));
        } else {
          session.debugHandleMessage(['EVENT', 'h-1', owned.toJson()]);
          session.debugHandleMessage(['EOSE', 'h-1']);
          await pending!;
        }
        if (pending != null) expect(await pending, transition == 'unchanged');
        // Drain continuations without an event-loop turn / provider refresh tick.
        await drainAdmission();
        expect(
          container.read(userCacheProvider.notifier).profileOwners,
          transition == 'unchanged' ? {agent.public: owner.public} : isEmpty,
        );
        if (boundary == 'search' && transition == 'unchanged') {
          await session.debugHandleConnected();
          final candidates = mentionCandidatesProvider((
            channelId: 'probe',
            query: 'agent',
          ));
          container.listen(candidates, (_, _) {});
          Future<void> accepts(bool allowed) async {
            await container.pump();
            await container.read(agentOwnersProvider.future);
            await container.pump();
            expect(
              container.read(candidates).map((c) => c.pubkey),
              allowed ? [agent.public] : isEmpty,
            );
          }

          final found = await container.read(search.future);
          await accepts(true);
          final oldAdmission = cache.captureAdmission();
          container.invalidate(userCacheProvider);
          final current = container.read(userCacheProvider.notifier);
          await accepts(false);
          expect(oldAdmission.isCurrent, isFalse);
          expect(current.profilePubkeys, isEmpty);
          expect(await container.read(agentOwnersProvider.future), isEmpty);
          expect(container.read(search).value, same(found));
        }
        if (boundary != 'search' && transition == 'community') {
          // New-context history still accepts valid authority after retirement.
          final recovery = container.read(userCacheProvider.notifier).refresh([
            agent.public,
          ]);
          const id = 'h-2';
          session.debugHandleMessage(['EVENT', id, owned.toJson()]);
          session.debugHandleMessage(['EOSE', id]);
          expect(await recovery, isTrue);
          expect(container.read(userCacheProvider.notifier).profileOwners, {
            agent.public: owner.public,
          });
        }
      });
    }
  }
}

Future<void> drainAdmission() async {
  for (var i = 0; i < 20; i++) {
    await Future<void>.value();
  }
}

class AdmissionCommunities extends CommunityListNotifier {
  AdmissionCommunities(this.initial);
  final Community initial;
  @override
  Future<List<Community>> build() async => [initial];
  void replace(Community community) => state = AsyncData([community]);
}

class AdmissionAuth extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.unauthenticated);
}

class AdmissionChannels extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() async => [];
}
