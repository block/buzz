import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

final fakeOwner = nostr.Keys('0' * 63 + '1');
final fakeAgent = nostr.Keys('0' * 63 + '2').public;
const fakeSubject = 'remote-owner';

/// Fakes only the Builderlab network boundary.
class FakeRegistry extends BuilderlabAccountClient {
  FakeRegistry({this.bound, this.signedIn = true});
  String? bound;
  bool signedIn;
  String subject = fakeSubject;
  bool failInstructions = false;
  bool rejectRegistration = false;
  Object deleteStatus = 1;
  final calls = <String>[];
  final bodies = <String, Map<String, dynamic>>{};
  List<Map<String, dynamic>> listed = [];

  @override
  BuilderlabAccount? get account => signedIn
      ? BuilderlabAccount(subject: subject, expiresAt: DateTime.utc(2030))
      : null;
  @override
  Future<BuilderlabAccount?> restore() async => account;
  @override
  Future<Map<String, dynamic>> currentIdentity() async => {
    'identity': {'pubkey_hex': bound},
  };
  @override
  Future<Map<String, dynamic>> post(
    String path,
    Map<String, dynamic> body,
  ) async {
    calls.add(path.split('/').last);
    bodies[path] = body;
    switch (path) {
      case '/v3/beekeeper/list-agents':
        return {'status': 1, 'agents': listed};
      case '/v3/beekeeper/register-agent':
        if (rejectRegistration) throw StateError('Registration rejected.');
        listed.add({
          'agent_pubkey': fakeAgent,
          'agent_id': 'agent',
          'agent_name': body['agent_name'],
          'status': 'BEEKEEPER_AGENT_STATUS_ACTIVE',
        });
        return {'status': 1, 'agent_pubkey': fakeAgent, 'agent_id': 'agent'};
      case '/v3/beekeeper/attest-agent':
        return {'status': 'ATTEST_AGENT_STATUS_ACTIVE'};
      case '/v3/beekeeper/update-agent':
        if (failInstructions) throw StateError('instructions unavailable');
        return {};
      case '/v3/beekeeper/add-buzz-subscription':
        return {'status': 1};
      case '/v3/beekeeper/delete-agent':
        expect(body, {'agent_id': 'remote-id'});
        return {'status': deleteStatus};
      default:
        throw StateError('Unexpected request: $path');
    }
  }
}

/// Fakes only relay publication; registry logic stays real.
class RelayFakeController extends RemoteAgentController {
  RelayFakeController(FakeRegistry account)
    : super(
        account: account,
        keys: AccountIdentityStore(),
        relayAuth: HostedRelayAuth(account),
      );
  bool rejectRemoval = false;
  FakeRegistry get registry => account as FakeRegistry;

  @override
  Future<void> publishPolicy(String relay, Map<String, dynamic> setup) async {
    registry.calls.add('publish-policy');
  }

  @override
  Future<void> removePolicy(String relay, String pubkey) async {
    registry.calls.add('remove-registration');
    if (rejectRemoval) throw StateError('Relay rejected deletion');
  }
}
