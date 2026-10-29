import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;

final fakeOwner = nostr.Keys('0' * 63 + '1');
final fakeAgentKeys = nostr.Keys('0' * 63 + '2');
final fakeAgent = fakeAgentKeys.public;
const fakeSubject = 'remote-owner';

/// Fakes only the Builderlab network boundary.
class FakeRegistry extends BuilderlabAccountClient {
  FakeRegistry({this.bound, bool signedIn = true}) : _signedIn = signedIn {
    _changes.value = signedIn ? fakeSubject : null;
  }
  String? bound;
  bool _signedIn;
  String _subjectValue = fakeSubject;
  final _changes = ValueNotifier<String?>(null);

  bool get signedIn => _signedIn;
  set signedIn(bool value) {
    _signedIn = value;
    _changes.value = value ? _subjectValue : null;
  }

  String get subject => _subjectValue;
  set subject(String value) {
    _subjectValue = value;
    if (_signedIn) _changes.value = value;
  }

  @override
  ValueListenable<String?> get subjectChanges => _changes;
  bool failInstructions = false;
  bool ignoreModelUpdate = false;
  List<Map<String, dynamic>> models = [
    {
      'id': 'test-model',
      'display_name': 'Test model',
      'supported_reasoning_efforts': ['low', 'high'],
    },
    {
      'id': 'other-model',
      'display_name': 'Other model',
      'supported_reasoning_efforts': ['max'],
    },
  ];
  Completer<void>? catalogGate;
  bool failListing = false;
  bool rejectRegistration = false;
  bool loseRegistration = false;
  bool refuseRegistration = false;
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
    Map<String, dynamic> body, {
    String? expectedSubject,
  }) async {
    if (expectedSubject != null && account?.subject != expectedSubject) {
      throw AccountFailure.differentAccount;
    }
    calls.add(path.split('/').last);
    bodies[path] = body;
    switch (path) {
      case '/v3/beekeeper/get-model-catalog':
        await catalogGate?.future;
        return {'models': models};
      case '/v3/beekeeper/list-agents':
        if (failListing) throw StateError('Registry unavailable.');
        return {'status': 1, 'agents': listed};
      case '/v3/beekeeper/register-agent':
        if (refuseRegistration) throw AccountFailure.rejected;
        if (rejectRegistration) {
          return {'status': 'REGISTER_AGENT_STATUS_NAME_TAKEN'};
        }
        listed.add({
          'agent_pubkey': fakeAgent,
          'agent_id': 'agent',
          'agent_name': body['agent_name'],
          'status': 'BEEKEEPER_AGENT_STATUS_ACTIVE',
        });
        // The server committed the identity, but the response never arrived.
        if (loseRegistration) throw StateError('Connection lost.');
        return {'status': 1, 'agent_pubkey': fakeAgent, 'agent_id': 'agent'};
      case '/v3/beekeeper/attest-agent':
        return {'status': 'ATTEST_AGENT_STATUS_ACTIVE'};
      case '/v3/beekeeper/update-agent':
        if (failInstructions) throw StateError('instructions unavailable');
        if (body.containsKey('model_id') && !ignoreModelUpdate) {
          listed = [
            for (final row in listed)
              if (row['agent_pubkey'] == body['agent_pubkey'])
                {
                  ...row,
                  'model_id': body['model_id'],
                  'reasoning_effort': body['reasoning_effort'],
                  'model_selection_status': 1,
                }
              else
                row,
          ];
        }
        return {};
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

/// A signed agent profile carrying [owner]'s attestation under [conditions].
NostrEvent signedAgentProfile(
  nostr.Keys agent,
  nostr.Keys owner, {
  String conditions = '',
  String content = '{}',
}) {
  final pubkey = agent.public;
  final digest = sha256
      .convert(utf8.encode('nostr:agent-auth:$pubkey:$conditions'))
      .toString();
  final auth = [
    'auth',
    owner.public,
    conditions,
    nostr.Schnorr.sign(secretKey: owner.secret, message: digest),
  ];
  return NostrEvent.fromJson(
    nostr.Event.from(
      kind: 0,
      content: content,
      secretKey: agent.secret,
      createdAt: 100,
      tags: [auth],
    ).toMap(),
  );
}
