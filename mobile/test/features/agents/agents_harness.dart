import 'package:buzz/shared/me/me_sync.dart';
import '../../shared/me/me_sync_test.dart' show FakeRelay;
import 'dart:async';

import 'package:buzz/features/agents/agents_sheet.dart';
import 'package:buzz/shared/agents/remote_agents.dart';
import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_provider.dart';
import 'package:buzz/shared/hosted/account_identity_store.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/remote_agents_fixture.dart';

/// Fakes the relay boundary: owner-attested profiles for registry agents.
class _Relay extends FakeRelay {
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => filter.kinds.contains(30078)
      ? events
      : [
          if (filter.authors?.contains(fakeAgent) ?? false)
            signedAgentProfile(
              fakeAgentKeys,
              fakeOwner,
              content: '{"name":"Scout"}',
            ),
        ];
}

/// Holds creation open so a test can observe the in-flight state.
class GatedController extends RelayFakeController {
  GatedController(super.account);
  Completer<void>? gate;
  String? submittedName;
  String? submittedInstructions;

  @override
  Future<bool> create(
    Community community,
    String name, {
    String instructions = '',
  }) {
    submittedName = name;
    submittedInstructions = instructions;
    final gate = this.gate;
    if (gate == null) {
      return super.create(community, name, instructions: instructions);
    }
    return run(() async {
      await gate.future;
      throw StateError('Creation failed. Retry.');
    });
  }
}

/// One signed-in owner whose saved key matches the account binding.
///
/// [AccountIdentityStore] serializes through a static queue whose futures
/// belong to the fake-async zone that created them, so each file holds only
/// one widget test that touches the store.
class AgentsHarness {
  AgentsHarness._(this.registry, this.controller, this.community);
  final FakeRegistry registry;
  final GatedController controller;
  final Community community;
  final relay = _Relay();

  static Future<AgentsHarness> create() async {
    FlutterSecureStorage.setMockInitialValues({});
    SharedPreferences.setMockInitialValues({});
    final registry = FakeRegistry(bound: fakeOwner.public);
    await AccountIdentityStore()
        .forSubject(registry.account!.subject)
        .create(fakeOwner.nsec);
    final controller = GatedController(registry);
    addTearDown(() {
      controller.dispose();
      controller.relayAuth.dispose();
      registry.dispose();
    });
    return AgentsHarness._(
      registry,
      controller,
      Community.create(
        name: 'Test',
        relayUrl: 'wss://test.${HostedEnvironment.communitySuffix}',
        pubkey: fakeOwner.public,
        nsec: fakeOwner.nsec,
      ),
    );
  }

  /// Opens the Agents sheet over a host page.
  Future<void> open(
    WidgetTester tester, {
    required List<String> directory,
    Widget? home,
  }) async {
    final prefs = await SharedPreferences.getInstance();
    final sync = MeSync(
      relay,
      RelayConfig(baseUrl: 'https://test', nsec: fakeOwner.nsec),
      prefs: prefs,
    );
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          builderlabAccountProvider.overrideWithValue(registry),
          remoteAgentControllerProvider.overrideWithValue(controller),
          relaySessionProvider.overrideWith(() => relay),
          meSyncProvider.overrideWithValue(sync),
          activeCommunityProvider.overrideWith((ref) async => community),
          agentDirectoryProvider.overrideWith(
            (ref) async => [
              for (final pubkey in directory)
                AgentDirectoryEntry(pubkey: pubkey),
            ],
          ),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home:
              home ??
              Builder(
                builder: (context) => Scaffold(
                  body: TextButton(
                    onPressed: () => showAgents(context),
                    child: const Text('Open Agents'),
                  ),
                ),
              ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    if (home == null) {
      await tester.tap(find.text('Open Agents'));
      await tester.pumpAndSettle();
    }
  }
}
