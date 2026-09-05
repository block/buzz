import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../crypto/nip_oa_test.dart' show authTag, profile;

void main() {
  final owner = nostr.Keys.generate();
  final agent = nostr.Keys.generate();
  final owned = profile(agent, [authTag(owner, agent.public)]);
  final revoked = profile(agent, [], createdAt: 101);
  final tampered = NostrEvent.fromJson({
    ...profile(agent, [authTag(owner, agent.public)], createdAt: 102).toJson(),
    'content': 'forged',
  });

  Future<Map<String, String>> owners(List<NostrEvent> events) async {
    final container = ProviderContainer(
      overrides: [
        agentDirectoryProvider.overrideWith(
          (ref) async => [
            AgentDirectoryEntry(pubkey: agent.public, displayName: 'Agent'),
          ],
        ),
        relaySessionProvider.overrideWith(() => _Profiles(events)),
      ],
    );
    try {
      return await container.read(agentOwnersProvider.future);
    } finally {
      container.dispose();
    }
  }

  test(
    'directory resolves only the latest ownership, never older fallback',
    () async {
      expect(await owners([owned]), {agent.public: owner.public});
      for (final latest in [revoked, tampered]) {
        for (final events in [
          [owned, latest],
          [latest, owned],
        ]) {
          expect(await owners(events), isEmpty);
          expect(
            directoryUsersFromProfileEvents(events).single.isAgent,
            isFalse,
          );
        }
      }
    },
  );

  test(
    'same-second owner revocation is deterministic across directory paths',
    () async {
      final other = profile(agent, []);
      final expected = owned.id.compareTo(other.id) < 0;
      for (final events in [
        [owned, other],
        [other, owned],
      ]) {
        expect((await owners(events)).containsKey(agent.public), expected);
        expect(
          directoryUsersFromProfileEvents(events).single.isAgent,
          expected,
        );
      }
    },
  );
}

class _Profiles extends RelaySessionNotifier {
  _Profiles(this.events);
  final List<NostrEvent> events;
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => events;
}
