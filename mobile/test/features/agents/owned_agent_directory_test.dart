import 'package:buzz/features/agents/owned_agent_directory.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import '../../helpers/remote_agents_fixture.dart';

void main() {
  test(
    'finds both owned agents without runtime entries or account sign-in',
    () async {
      final desktop = nostr.Keys('0' * 63 + '3');
      final outsider = nostr.Keys('0' * 63 + '4');
      final registrations = [fakeAgent, desktop.public, outsider.public]
          .map(
            (key) => NostrEvent.fromJson(
              nostr.Event.from(
                kind: 30177,
                content: key == fakeAgent ? '{"model":"Claude Sonnet"}' : '{}',
                secretKey: fakeOwner.secret,
                createdAt: 1,
                tags: [
                  ['d', key],
                ],
              ).toMap(),
            ),
          )
          .toList();
      final profiles = [
        signedAgentProfile(
          fakeAgentKeys,
          fakeOwner,
          content: '{"name":"Remote"}',
        ),
        signedAgentProfile(desktop, fakeOwner, content: '{"name":"Desktop"}'),
        signedAgentProfile(outsider, desktop, content: '{"name":"Not owned"}'),
      ];
      final found = await loadOwnedAgentDirectory(fakeOwner.public, (
        filter,
      ) async {
        if (filter.kinds.contains(30177)) {
          expect(filter.authors, [fakeOwner.public]);
          return registrations;
        }
        expect(filter.kinds, [0]);
        return profiles;
      });
      expect(found.first.model, 'Claude Sonnet');
      expect(found.last.model, isNull);
      expect(found.map((entry) => entry.displayName), ['Remote', 'Desktop']);
      expect(found.map((entry) => entry.pubkey), [fakeAgent, desktop.public]);
    },
  );
}
