import 'package:buzz/shared/me/me_sync.dart';
import '../../shared/me/me_sync_test.dart' show FakeRelay;
import 'dart:async';
import 'package:buzz/shared/relay/relay_author_provider.dart';
import 'package:nostr/nostr.dart' as nostr;

import 'package:buzz/features/agents/agents_page.dart';
import 'package:buzz/shared/agents/owned_agent_directory.dart';
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
import 'package:hooks_riverpod/misc.dart';
import 'package:shared_preferences/shared_preferences.dart';

import '../../helpers/remote_agents_fixture.dart';

final existingDesktopKeys = nostr.Keys('0' * 63 + '3');
final otherAgentKeys = nostr.Keys('0' * 63 + '4');
final otherOwnerKeys = nostr.Keys('0' * 63 + '5');

/// The owner's agent that only an earlier Buzz runs: its registration has no
/// `runtime`, so the Agents page leaves it out as desktop does.
final oldAppAgentKeys = nostr.Keys('0' * 63 + '6');

/// Fakes the relay boundary: owner-attested profiles for registry agents.
class _Relay extends FakeRelay {
  _Relay(this.harness);
  final AgentsHarness harness;

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    final handler = harness.onQuery;
    return handler == null
        ? super.queryRelay(filters, timeout: timeout)
        : handler(filters.single);
  }

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async {
    if ((filter.authors?.contains(fakeAgent) ?? false) && harness.relayDown) {
      throw TimeoutException('relay unavailable');
    }
    if (filter.kinds.contains(30177)) {
      return [
        for (final pubkey in harness.registered)
          NostrEvent.fromJson(
            nostr.Event.from(
              kind: 30177,
              content: '{"name":"Agent"}',
              secretKey: fakeOwner.secret,
              createdAt: 1,
              tags: [
                ['d', pubkey],
                if (pubkey != oldAppAgentKeys.public)
                  ['runtime', 'block.claude-code/claude-code'],
              ],
            ).toMap(),
          ),
      ];
    }
    return filter.kinds.contains(30078)
        ? events
        : [
            if (filter.authors?.contains(oldAppAgentKeys.public) ?? false)
              signedAgentProfile(
                oldAppAgentKeys,
                fakeOwner,
                content: '{"name":"Old app agent"}',
              ),
            if (filter.authors?.contains(existingDesktopKeys.public) ?? false)
              signedAgentProfile(
                existingDesktopKeys,
                fakeOwner,
                content: '{"name":"My desktop agent"}',
              ),
            if (filter.authors?.contains(otherAgentKeys.public) ?? false)
              signedAgentProfile(
                otherAgentKeys,
                otherOwnerKeys,
                content: '{"name":"Other owner agent"}',
              ),
            if (filter.authors?.contains(fakeAgent) ?? false)
              signedAgentProfile(
                fakeAgentKeys,
                fakeOwner,
                content: '{"name":"Scout"}',
              ),
          ];
  }
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
    String? pictureUrl,
    String? modelId,
    String? reasoningEffort,
  }) {
    submittedName = name;
    submittedInstructions = instructions;
    final gate = this.gate;
    if (gate == null) {
      return super.create(
        community,
        name,
        instructions: instructions,
        pictureUrl: pictureUrl,
        modelId: modelId,
        reasoningEffort: reasoningEffort,
      );
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
  Future<List<NostrEvent>> Function(NostrFilter)? onQuery;
  AgentsHarness._(this.registry, this.controller, this.community);
  final FakeRegistry registry;
  final GatedController controller;
  final Community community;
  late final relay = _Relay(this);

  /// Fails agent profile fetches until the relay recovers.
  bool relayDown = false;
  bool authorityDown = false;

  /// Agents the owner registered in this community (kind:30177).
  List<String> registered = const [];

  /// [relayUrl] defaults to a hosted community that supports remote agents.
  static Future<AgentsHarness> create({String? relayUrl}) async {
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
        relayUrl: relayUrl ?? 'wss://test.${HostedEnvironment.communitySuffix}',
        pubkey: fakeOwner.public,
        nsec: fakeOwner.nsec,
      ),
    );
  }

  /// Opens the Agents sheet over a host page. [directory] agents are both
  /// registered by the owner and listed in the relay agent directory; the
  /// owner's old-app agent is always registered too.
  Future<void> open(
    WidgetTester tester, {
    required List<String> directory,
    Widget? home,
    List<Override> overrides = const [],
  }) async {
    registered = [...directory, oldAppAgentKeys.public];
    final prefs = await SharedPreferences.getInstance();
    final sync = MeSync(
      relay,
      RelayConfig(baseUrl: 'https://test', nsec: fakeOwner.nsec),
      prefs: prefs,
    );
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          ownedAgentDirectoryProvider.overrideWith(
            (ref) =>
                loadOwnedAgentDirectory(fakeOwner.public, relay.fetchHistory),
          ),
          builderlabAccountProvider.overrideWithValue(registry),
          remoteAgentControllerProvider.overrideWithValue(controller),
          relaySessionProvider.overrideWith(() => relay),
          relayAuthorProvider(community.relayUrl).overrideWith((ref) async {
            if (authorityDown) throw StateError('Relay metadata offline');
            return 'e' * 64;
          }),
          meSyncProvider.overrideWithValue(sync),
          activeCommunityProvider.overrideWith((ref) async => community),
          agentDirectoryProvider.overrideWith(
            (ref) async => [
              for (final pubkey in directory)
                AgentDirectoryEntry(pubkey: pubkey),
            ],
          ),
          ...overrides,
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          builder: (context, child) => MediaQuery(
            data: MediaQuery.of(context).copyWith(disableAnimations: true),
            child: child!,
          ),
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
