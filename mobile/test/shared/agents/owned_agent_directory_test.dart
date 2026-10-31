import 'package:buzz/shared/agents/owned_agent_directory.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import '../../helpers/remote_agents_fixture.dart';

NostrEvent registration(
  String agent, {
  String content = '{"name":"Agent"}',
  String? runtime = 'block.builderlab/builderlab',
  int createdAt = 1,
}) => NostrEvent.fromJson(
  nostr.Event.from(
    kind: 30177,
    content: content,
    secretKey: fakeOwner.secret,
    createdAt: createdAt,
    tags: [
      ['d', agent],
      if (runtime != null) ['runtime', runtime],
    ],
  ).toMap(),
);

void main() {
  test('lists owned agents as desktop does, without runtime directory '
      'entries or account sign-in', () async {
    final desktop = nostr.Keys('0' * 63 + '3');
    final outsider = nostr.Keys('0' * 63 + '4');
    final oldApp = nostr.Keys('0' * 63 + '5');
    final reverted = nostr.Keys('0' * 63 + '6');
    final imported = nostr.Keys('0' * 63 + '7');
    final registrations = [
      registration(
        fakeAgent,
        content: '{"name":"Remote","model":"Claude Sonnet"}',
      ),
      registration(desktop.public, runtime: 'block.claude-code/claude-code'),
      registration(outsider.public),
      // An earlier Buzz wrote no runtime; desktop lists it once imported.
      registration(oldApp.public, runtime: null),
      registration(imported.public, runtime: null),
      registration(imported.public, createdAt: 2),
      // Only the newest registration counts.
      registration(reverted.public),
      registration(reverted.public, runtime: null, createdAt: 2),
    ];
    final profiles = [
      signedAgentProfile(
        fakeAgentKeys,
        fakeOwner,
        content: '{"name":"Remote"}',
      ),
      signedAgentProfile(desktop, fakeOwner, content: '{"name":"Desktop"}'),
      signedAgentProfile(outsider, desktop, content: '{"name":"Not owned"}'),
      for (final keys in [oldApp, reverted, imported])
        signedAgentProfile(keys, fakeOwner, content: '{"name":"Owned"}'),
    ];
    final found = await loadOwnedAgentDirectory(fakeOwner.public, (
      filter,
    ) async {
      if (filter.kinds.contains(30177)) {
        expect(filter.authors, [fakeOwner.public]);
        return registrations;
      }
      expect(filter.kinds, [0]);
      return profiles
          .where((profile) => filter.authors!.contains(profile.pubkey))
          .toList();
    });
    expect(found.first.model, 'Claude Sonnet');
    expect(found.map((entry) => entry.pubkey), [
      fakeAgent,
      desktop.public,
      imported.public,
    ]);
  });

  test('a registration is listed only with a type runtime and a name', () {
    final agent = fakeAgent;
    final cases = <(String, NostrEvent, bool)>[
      ('valid', registration(agent), true),
      ('no runtime', registration(agent, runtime: null), false),
      ('runtime without a type', registration(agent, runtime: 'goose'), false),
      ('uppercase runtime', registration(agent, runtime: 'Block/Goose'), false),
      ('empty name', registration(agent, content: '{"name":"  "}'), false),
      ('no name', registration(agent, content: '{}'), false),
      ('not JSON', registration(agent, content: 'instance'), false),
      ('JSON list', registration(agent, content: '[]'), false),
    ];
    for (final (label, event, listed) in cases) {
      expect(isListedAgentRegistration(event), listed, reason: label);
    }
  });
}
