import 'package:buzz/shared/agents/first_agent_bestie.dart';
import 'package:buzz/shared/me/default_agent.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import '../../helpers/remote_agents_fixture.dart';
import '../me/me_sync_test.dart' show FakeRelay;
import 'owned_agent_directory_test.dart' show registration;

class _Relay extends FakeRelay {
  List<nostr.Keys> agents = [fakeAgentKeys];
  bool fail = false;
  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if (fail) throw StateError('Offline');
    if (filter.kinds.contains(30177)) {
      return agents.map((agent) => registration(agent.public)).toList();
    }
    if (filter.kinds.contains(0)) {
      return agents
          .map((agent) => signedAgentProfile(agent, fakeOwner))
          .toList();
    }
    return super.fetchHistory(filter, timeout: timeout);
  }
}

void main() {
  MeSync sync(_Relay relay) =>
      MeSync(relay, RelayConfig(baseUrl: 'https://test', nsec: fakeOwner.nsec));
  test(
    'first created agent becomes Bestie and retries are idempotent',
    () async {
      final relay = _Relay();
      final service = sync(relay);
      await makeFirstAgentBestie(service, fakeAgent);
      expect(decodeDefaultAgent(relay.events.single, service), fakeAgent);
      await makeFirstAgentBestie(service, fakeAgent);
      expect(relay.events, hasLength(1));
    },
  );
  test('a second agent does not become Bestie automatically', () async {
    final relay = _Relay()..agents.add(nostr.Keys('0' * 63 + '3'));
    await makeFirstAgentBestie(sync(relay), fakeAgent);
    expect(relay.writes, 0);
  });
  test(
    'unknown directory and incomplete registration remain retryable',
    () async {
      final relay = _Relay()..fail = true;
      await expectLater(
        makeFirstAgentBestie(sync(relay), fakeAgent),
        throwsStateError,
      );
      relay.fail = false;
      relay.agents = [];
      await expectLater(
        makeFirstAgentBestie(sync(relay), fakeAgent),
        throwsStateError,
      );
      expect(relay.writes, 0);
    },
  );
}
