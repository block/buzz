import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:crypto/crypto.dart';
import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';
import 'package:uuid/uuid.dart';

import '../community/community.dart';
import '../community/community_provider.dart';
import '../crypto/nip_oa.dart';
import '../hosted/account_identity_store.dart';
import '../hosted/builderlab_account.dart';
import '../hosted/device_identity_source.dart';
import '../hosted/hosted_environment.dart';
import '../hosted/hosted_relay_auth.dart';
import '../relay/nostr_filters.dart';
import '../relay/nostr_models.dart';
import '../relay/relay_provider.dart';
import '../relay/relay_session.dart';
import '../relay/signed_event_relay.dart';
import '../storage/secure_storage.dart';

/// Remote agents run on Builderlab, so they exist only in builds with
/// Builderlab enabled and only in hosted communities this build supports.
bool supportsRemoteAgents(Community? community) =>
    HostedEnvironment.enabled &&
    community != null &&
    community.pubkey != null &&
    HostedEnvironment.supportsRelay(Uri.parse(community.relayUrl));

/// Restores a saved Builderlab session once, so a cold start with a stored
/// session shows remote agents without waiting for another sign-in path.
final builderlabRestoreProvider = FutureProvider<void>((ref) async {
  try {
    await ref.watch(builderlabAccountProvider).restore();
  } on Object {
    // Unreachable or rejected sessions stay signed out; sign-in recovers them.
  }
});

/// Whether a Builderlab account is signed in, updating on sign-in and out.
final builderlabSignedInProvider = Provider<bool>((ref) {
  if (HostedEnvironment.enabled) ref.watch(builderlabRestoreProvider);
  final subject = ref.watch(builderlabAccountProvider).subjectChanges;
  void changed() => ref.invalidateSelf();
  subject.addListener(changed);
  ref.onDispose(() => subject.removeListener(changed));
  return subject.value != null;
});

/// Remote agents appear only in supported communities while signed in to
/// Builderlab. Otherwise remote-agent listing, creation and deletion are
/// hidden; the general Agents directory stays available.
final remoteAgentsEnabledProvider = Provider<bool>(
  (ref) =>
      ref.watch(builderlabSignedInProvider) &&
      supportsRemoteAgents(ref.watch(activeCommunityProvider).value),
);

final remoteAgentControllerProvider = Provider<RemoteAgentController>((ref) {
  final controller = RemoteAgentController(
    account: ref.watch(builderlabAccountProvider),
    keys: AccountIdentityStore(),
    relayAuth: ref.watch(hostedRelayAuthProvider),
  );
  ref.onDispose(controller.dispose);
  return controller;
});

/// The active community's remote agents that the signed-in owner's registry
/// reports ACTIVE. Remote workers wake on messages, so this means available,
/// not socket presence.
final availableRemoteAgentsProvider = FutureProvider<Set<String>>((ref) async {
  final community = ref.watch(activeCommunityProvider).asData?.value;
  if (!supportsRemoteAgents(community)) return const {};
  // A reconnect retries a registry load that failed.
  ref.watch(relaySessionProvider.select((session) => session.status));
  final controller = ref.watch(remoteAgentControllerProvider);
  final active = controller.activeAgents;
  final revision = controller.registrationRevision;
  void changed() {
    if (!setEquals(controller.activeAgents, active) ||
        controller.registrationRevision != revision) {
      ref.invalidateSelf();
    }
  }

  controller.addListener(changed);
  ref.onDispose(() => controller.removeListener(changed));
  // Loading notifies listeners, which must not happen during this build.
  scheduleMicrotask(() => controller.load(community!.relayUrl));
  if (active.isEmpty) return const {};
  // The registry is account-wide; only agents live on this relay count.
  return liveCommunityAgents(
    active,
    controller.owner,
    ref.read(relaySessionProvider.notifier).fetchHistory,
  );
});

/// Registry identities with verified owner profiles on this relay. Account-wide
/// registry entries alone are not membership. A deletion interrupted after its
/// relay registration was removed still counts, so it can be retried.
Future<Set<String>> ownedCommunityAgents(
  Set<String> keys,
  String? owner,
  Future<List<NostrEvent>> Function(NostrFilter) fetch,
) async {
  if (keys.isEmpty || owner == null) return {};
  final events = await fetch(NostrFilters.profilesBatch(keys.toList()));
  return {
    for (final event in events)
      if (keys.contains(event.pubkey) &&
          verifiedProfileOaOwnerPubkey(event) == owner)
        event.pubkey,
  };
}

/// Owned agents whose kind 30177 registration is live on this relay, so they
/// can answer here. Only these are presented as available.
Future<Set<String>> liveCommunityAgents(
  Set<String> keys,
  String? owner,
  Future<List<NostrEvent>> Function(NostrFilter) fetch,
) async {
  final owned = await ownedCommunityAgents(keys, owner, fetch);
  if (owned.isEmpty) return {};
  final registrations = await fetch(
    NostrFilter(
      kinds: const [30177],
      authors: [owner!],
      tags: {'#d': owned.toList()},
    ),
  );
  return {
    for (final event in registrations)
      if (event.kind == 30177 && event.pubkey == owner)
        for (final tag in event.tags)
          if (tag.length > 1 && tag[0] == 'd' && owned.contains(tag[1])) tag[1],
  };
}

/// Owner-side registry client for Builderlab remote agents, using the same
/// account and Beekeeper contracts as desktop.
class RemoteAgentController extends ChangeNotifier {
  RemoteAgentController({
    required this.account,
    required AccountIdentityStore keys,
    required this.relayAuth,
  }) : _keys = keys {
    account.subjectChanges.addListener(_accountChanged);
  }

  final BuilderlabAccountClient account;
  final AccountIdentityStore _keys;
  final HostedRelayAuth relayAuth;

  bool busy = false;
  int _operation = 0;

  /// Why the last [run] failed, or empty.
  String failure = '';
  String? owner;
  List<Map<String, dynamic>> agents = [];
  Map<String, dynamic>? pending;
  Set<String> deletedAgents = {};
  String? _scope;
  String? _loadedRelay;
  bool _disposed = false;

  /// The current account's own identity slot.
  AccountIdentityStore get keys {
    final subject = account.account?.subject;
    if (subject == null) throw AccountFailure.signedOut;
    return _keys.forSubject(subject);
  }

  /// Empty until the signed-in account's own registry has loaded.
  Set<String> get activeAgents => _scope == null || _scope != _currentScope
      ? const {}
      : {
          for (final agent in agents)
            if (agent['status'] == 'BEEKEEPER_AGENT_STATUS_ACTIVE' ||
                agent['status'] == 2)
              agent['agent_pubkey'] as String,
        };

  String? get _currentScope {
    final subject = account.account?.subject;
    return subject == null
        ? null
        : 'remote-agents:${HostedEnvironment.origin.host}:$subject';
  }

  String get _storageScope => _currentScope ?? (throw AccountFailure.signedOut);

  /// Forgets the previous account's registry before showing another's.
  void _clearAccountState() {
    agents = [];
    owner = null;
    pending = null;
    deletedAgents = {};
    listingFailed = false;
    _scope = null;
  }

  /// Drops another account's registry as soon as the account changes.
  void _accountChanged() {
    if (_scope == null || _scope == _currentScope) return;
    _clearAccountState();
    _loadedRelay = null;
    _notify();
  }

  /// Loads the registry once per community; opening Agents always refreshes.
  /// A failed load is retried on the next call.
  Future<void> load(String relay) async {
    if (_disposed || busy || _loadedRelay == relay) return;
    _loadedRelay = relay;
    if (!await run(refresh) && _loadedRelay == relay) _loadedRelay = null;
  }

  /// Runs one operation at a time and turns its failure into [failure].
  Future<bool> run(Future<void> Function() action) async {
    if (busy || _disposed) return false;
    busy = true;
    account.lastRejectedRequest = null;
    failure = '';
    final operation = ++_operation;
    notifyListeners();
    try {
      await action();
      return true;
    } catch (error) {
      // Account failures never carry credentials or response bodies.
      failure = switch (error) {
        AccountFailure.signedOut => 'Sign in to Builderlab again.',
        AccountFailure.rejected when account.lastRejectedRequest != null =>
          'Builderlab rejected ${account.lastRejectedRequest!.path.split('/').last} '
              '(HTTP ${account.lastRejectedRequest!.status}). Retry this setup.',
        AccountFailure() => 'Builderlab: ${error.name}. Try again.',
        AccountIdentityFailure() => 'Buzz identity: ${error.name}.',
        StateError() => error.message,
        _ => 'Operation failed (${error.runtimeType}). Try again.',
      };
      return false;
    } finally {
      if (operation == _operation && !_disposed) {
        busy = false;
        notifyListeners();
      }
    }
  }

  void _notify() {
    if (!_disposed) notifyListeners();
  }

  /// Bumped when this device publishes or removes a kind 30177 registration,
  /// which changes availability even when the registry's ACTIVE set does not.
  int registrationRevision = 0;

  void _registrationsChanged() {
    registrationRevision++;
    _notify();
  }

  /// Whether the last registry listing failed, so an outage never reads as
  /// an empty registry.
  bool listingFailed = false;

  Future<void> refresh() async {
    try {
      await _refresh();
      listingFailed = false;
    } catch (_) {
      listingFailed = true;
      rethrow;
    }
  }

  Future<void> _refresh() async {
    final session = account.account ?? await account.restore();
    if (session == null) {
      _clearAccountState();
      return;
    }
    await ensureBinding();
    final result = await account.post('/v3/beekeeper/list-agents', {});
    requireStatus(result, 'SUCCESS', 1);
    final rows = result['agents'] ?? [];
    if (rows is! List) throw StateError('Invalid agent listing.');
    final listed = [
      for (final row in rows) Map<String, dynamic>.from(row as Map),
    ];
    final prefs = await SharedPreferences.getInstance();
    final retries = Map<String, dynamic>.from(
      jsonDecode(prefs.getString('$_storageScope:delete-retries') ?? '{}')
          as Map,
    );
    // A deletion interrupted after its relay registration was removed keeps
    // its row so Delete can be retried.
    for (final retry in retries.values) {
      final row = Map<String, dynamic>.from(retry as Map);
      if (!listed.any(
        (agent) => agent['agent_pubkey'] == row['agent_pubkey'],
      )) {
        listed.add(row);
      }
    }
    agents = listed
        .where((row) => !deletedAgents.contains(row['agent_pubkey']))
        .toList();
  }

  /// Requires this account's saved key to be its current server binding.
  Future<void> ensureBinding() async {
    final subject = account.account?.subject;
    if (subject == null) throw AccountFailure.signedOut;
    if (_scope != _storageScope) _clearAccountState();
    final slot = keys;
    var local = await slot.read();
    final response = await account.currentIdentity();
    if (account.account?.subject != subject) {
      throw AccountFailure.differentAccount;
    }
    if (response['error'] != null) throw AccountFailure.invalidResponse;
    final bound = (response['identity'] as Map?)?['pubkey_hex'];
    // Reconnect restores the login only. A paired/legacy community can already
    // hold the account's bound identity without an account-scoped key slot.
    // Adopt only that exact server-confirmed key; never generate or rebind one.
    if (local == null &&
        bound is String &&
        RegExp(r'^[0-9a-f]{64}$').hasMatch(bound)) {
      void checkCurrent() {
        if (_disposed || account.account?.subject != subject) {
          throw AccountFailure.differentAccount;
        }
      }

      final device = DeviceIdentitySource();
      final choices = await device.choices();
      checkCurrent();
      if (choices.any((choice) => choice.pubkey == bound)) {
        local = await device.adopt(bound, slot, checkCurrent: checkCurrent);
      } else {
        final paired = await _keys.forPairing(bound).read();
        checkCurrent();
        if (paired?.pubkey == bound) {
          local = await slot.adoptSaved(bound, checkCurrent: checkCurrent);
        }
      }
      checkCurrent();
    }
    if (local == null) throw AccountIdentityFailure.absent;
    if (bound != local.pubkey) throw AccountIdentityFailure.mismatch;
    owner = local.pubkey;
    final scope = _storageScope;
    if (_scope == scope) return;
    final prefs = await SharedPreferences.getInstance();
    final raw = prefs.getString('$scope:pending');
    final restored = raw == null
        ? null
        : Map<String, dynamic>.from(jsonDecode(raw) as Map);
    if (restored?['hasInstructions'] == true) {
      final instructions = await accountSecureStorage.read(
        key: '$scope:instructions:${restored!['request']}',
      );
      if (instructions == null) {
        // Preferences can outlive device-only secure storage, for example
        // after a device restore. Keep the setup and ask for them again.
        restored['instructionsMissing'] = true;
      } else {
        restored['instructions'] = instructions;
      }
    }
    deletedAgents = prefs.getStringList('$scope:deleted')?.toSet() ?? {};
    pending = restored;
    _scope = scope;
  }

  /// Loads the authenticated service's supported model and effort choices.
  Future<List<RemoteAgentModel>> modelCatalog() async {
    final subject = account.account?.subject;
    if (subject == null) throw AccountFailure.signedOut;
    final result = await account.post(
      '/v3/beekeeper/get-model-catalog',
      {},
      expectedSubject: subject,
    );
    if (account.account?.subject != subject) {
      throw AccountFailure.differentAccount;
    }
    final rows = result['models'];
    if (rows is! List) throw StateError('Could not load available models.');
    return rows.map((row) => RemoteAgentModel.fromJson(row)).toList();
  }

  /// Saves only an active agent owned by this account, then verifies readback.
  Future<bool> updateModel({
    required Community community,
    required String pubkey,
    required RemoteAgentModel model,
    required String effort,
    required bool Function() isCurrent,
  }) => run(() async {
    final subject = account.account?.subject;
    void check() {
      if (subject == null ||
          account.account?.subject != subject ||
          !isCurrent()) {
        throw StateError('Community or account changed. Reopen Agents.');
      }
    }

    check();
    if (!HostedEnvironment.supportsRelay(Uri.parse(community.relayUrl))) {
      throw StateError('This community does not support remote agents.');
    }
    await ensureBinding();
    check();
    if (owner != community.pubkey) throw AccountIdentityFailure.mismatch;
    if (!activeAgents.contains(pubkey)) {
      throw StateError('Refresh Agents before changing this model.');
    }
    final catalog = await modelCatalog();
    check();
    if (!catalog.any(
      (choice) => choice.id == model.id && choice.efforts.contains(effort),
    )) {
      throw StateError(
        'This model or reasoning effort is no longer available.',
      );
    }
    final result = await account.post('/v3/beekeeper/update-agent', {
      'agent_pubkey': pubkey,
      'model_id': model.id,
      'reasoning_effort': effort,
    }, expectedSubject: subject);
    check();
    if (result['error'] != null) throw AccountFailure.rejected;
    final readback = await account.post(
      '/v3/beekeeper/list-agents',
      {},
      expectedSubject: subject,
    );
    check();
    requireStatus(readback, 'SUCCESS', 1);
    final rows = readback['agents'];
    if (rows is! List) throw StateError('Invalid agent listing.');
    agents = [
      for (final row in rows) Map<String, dynamic>.from(row as Map),
    ].where((row) => !deletedAgents.contains(row['agent_pubkey'])).toList();
    final saved = agents
        .where((row) => row['agent_pubkey'] == pubkey)
        .firstOrNull;
    if (saved?['model_id'] != model.id ||
        saved?['reasoning_effort'] != effort ||
        !(saved?['model_selection_status'] == 1 ||
            saved?['model_selection_status'] ==
                'MODEL_SELECTION_STATUS_VALID')) {
      throw StateError(
        'The server has not confirmed this model selection. Refresh and try again.',
      );
    }
  });

  Future<void> _savePending(Map<String, dynamic> value) async {
    final scope = _storageScope;
    final prefs = await SharedPreferences.getInstance();
    final persisted = Map<String, dynamic>.from(value)
      ..remove('instructions')
      ..remove('hasInstructions');
    final instructions = value['instructions'] as String?;
    if (instructions != null && instructions.isNotEmpty) {
      await accountSecureStorage.write(
        key: '$scope:instructions:${value['request']}',
        value: instructions,
      );
      persisted['hasInstructions'] = true;
    }
    if (!await prefs.setString('$scope:pending', jsonEncode(persisted))) {
      throw StateError('Could not save unfinished setup.');
    }
    pending = value;
  }

  /// Rejects selections absent from the current server catalog.
  Future<void> validateModel(String modelId, String effort) async {
    final catalog = await modelCatalog();
    if (!catalog.any(
      (model) => model.id == modelId && model.efforts.contains(effort),
    )) {
      throw StateError(
        'Choose a model and reasoning effort offered in the model catalog.',
      );
    }
  }

  /// Registers, activates and enrolls an agent in [community]. A failed
  /// attempt keeps its identity so the next call resumes the same agent.
  Future<bool> create(
    Community community,
    String name, {
    String instructions = '',
    String? pictureUrl,
    String? modelId,
    String? reasoningEffort,
  }) => run(() async {
    if (!HostedEnvironment.supportsRelay(Uri.parse(community.relayUrl))) {
      throw StateError('This community does not support remote agents.');
    }
    await ensureBinding();
    if (community.pubkey != owner) {
      throw StateError(
        'Sign in with the Builderlab account you use in this community.',
      );
    }
    final scope = _storageScope;
    final resuming = pending != null;
    final selectedModel = modelId ?? pending?['model_id'] as String?;
    final selectedEffort =
        reasoningEffort ?? pending?['reasoning_effort'] as String?;
    if (selectedModel == null ||
        selectedModel.isEmpty ||
        selectedEffort == null ||
        selectedEffort.isEmpty) {
      throw StateError('Choose a model and reasoning effort.');
    }
    if (pending == null) {
      final trimmed = name.trim();
      if (!RegExp(r'^[\w .-]{1,64}$').hasMatch(trimmed)) {
        throw StateError(
          'Use 1 to 64 letters, numbers, spaces, dots, hyphens or underscores.',
        );
      }
      if (instructions.trim().length > 20000) {
        throw StateError('Use at most 20,000 characters for instructions.');
      }
      await validateModel(selectedModel, selectedEffort);
      await _savePending({
        'name': trimmed,
        'instructions': instructions.trim(),
        'picture': ?pictureUrl,
        'community': community.relayUrl,
        'owner': owner,
        'request': const Uuid().v4(),
      });
    }
    var setup = {
      ...pending!,
      'model_id': selectedModel,
      'reasoning_effort': selectedEffort,
    };
    // A registration whose response was lost may be replayed from any
    // community, so the account learns its identity even if the original
    // community is unavailable. Everything after it needs that community.
    final elsewhere = setup['community'] != community.relayUrl;
    final unknownIdentity =
        setup['pubkey'] == null && setup['registering'] == true;
    if (setup['owner'] != owner || (elsewhere && !unknownIdentity)) {
      throw StateError('Finish the pending setup in its original community.');
    }
    if (resuming) await validateModel(selectedModel, selectedEffort);
    await _savePending(setup);
    if (setup['instructionsMissing'] == true) {
      if (instructions.trim().length > 20000) {
        throw StateError('Use at most 20,000 characters for instructions.');
      }
      setup = {...setup, 'instructions': instructions.trim()}
        ..remove('instructionsMissing');
      await _savePending(setup);
    }
    if (setup['pubkey'] == null) {
      // Once sent, the server may allocate an identity even if the response is
      // lost. Only Resume, with the same idempotency key, can recover it.
      setup = {...setup, 'registering': true};
      await _savePending(setup);
      // An explicit rejection allocated nothing, so it can be discarded.
      Future<void> refused() async {
        setup = {...setup}..remove('registering');
        await _savePending(setup);
      }

      final Map<String, dynamic> result;
      try {
        result = await account.post('/v3/beekeeper/register-agent', {
          'agent_name': setup['name'],
          if (setup['picture'] != null) 'agent_picture_url': setup['picture'],
          'idempotency_key': setup['request'],
        });
      } on AccountFailure catch (failure) {
        if (failure == AccountFailure.rejected) await refused();
        rethrow;
      }
      if (!hasStatus(result, 'UNATTESTED', 1) &&
          result['status'] != null &&
          result['agent_pubkey'] == null) {
        await refused();
      }
      requireStatus(result, 'UNATTESTED', 1);
      final pubkey = result['agent_pubkey'];
      if (pubkey is! String ||
          !RegExp(r'^[0-9a-f]{64}$').hasMatch(pubkey) ||
          result['agent_id'] is! String) {
        throw StateError('Invalid registration response. Retry this setup.');
      }
      setup = {...setup, 'pubkey': pubkey, 'id': result['agent_id']};
      await _savePending(setup);
    }
    if (elsewhere) {
      throw StateError(
        'Agent identity recovered. Finish the setup in its original community.',
      );
    }
    final pubkey = setup['pubkey'] as String;
    final auth = await keys.withKeys(
      (key) async => signOwnerAuthorization(key, pubkey),
    );
    final result = await account.post('/v3/beekeeper/attest-agent', {
      'agent_pubkey': pubkey,
      'owner_auth_tag_json': jsonEncode(auth),
      'community_url': RelayConfig(baseUrl: community.relayUrl).wsUrl,
    });
    requireStatus(result, 'ACTIVE', 1);
    await account.post('/v3/beekeeper/update-agent', {
      'agent_pubkey': pubkey,
      'agent_instructions': setup['instructions'] ?? '',
      'model_id': setup['model_id'],
      'reasoning_effort': setup['reasoning_effort'],
    });
    await publishPolicy(community.relayUrl, setup);
    _registrationsChanged();
    // Retain the completed registration if the listing refresh fails, so Retry
    // resumes the same identity instead of creating another agent.
    await refresh();
    // Drop the record before its instructions, so an interrupted cleanup never
    // leaves a pending setup whose instructions are gone.
    final prefs = await SharedPreferences.getInstance();
    if (!await prefs.remove('$scope:pending')) {
      throw StateError('Could not clear the finished setup. Retry.');
    }
    pending = null;
    await accountSecureStorage.delete(
      key: '$scope:instructions:${setup['request']}',
    );
  });

  /// Whether [pending] provably has no server identity: registration was never
  /// sent, or the server explicitly rejected it.
  bool get canDiscardPending =>
      pending != null &&
      pending!['pubkey'] == null &&
      pending!['registering'] != true;

  /// Abandons an unfinished setup that never got a server identity, such as
  /// one whose registration the server rejects. Setups that have or may have
  /// an identity can only be resumed, so no registered agent is orphaned.
  Future<bool> discardPending() => run(() async {
    final setup = pending;
    if (setup == null) return;
    if (!canDiscardPending) {
      throw StateError('This setup may already have an agent. Resume it.');
    }
    final scope = _storageScope;
    final prefs = await SharedPreferences.getInstance();
    if (!await prefs.remove('$scope:pending')) {
      throw StateError('Could not discard unfinished setup.');
    }
    pending = null;
    await accountSecureStorage.delete(
      key: '$scope:instructions:${setup['request']}',
    );
  });

  /// Publishes the owner's agent registration after attest-agent confirms
  /// activation and enrollment in the requested community.
  Future<void> publishPolicy(String relay, Map<String, dynamic> setup) async {
    final pubkey = setup['pubkey'] as String;
    final prefs = await SharedPreferences.getInstance();
    if (prefs.getStringList('$_storageScope:deleting')?.contains(pubkey) ==
        true) {
      throw StateError('Agent deletion is pending. Retry Delete.');
    }
    await keys.withKeys((key) async {
      // Saved communities may store an https relay URL; sockets need wss.
      final config = RelayConfig(baseUrl: relay, nsec: key.nsec);
      await submitSignedEventOnce(
        wsUrl: config.wsUrl,
        headers: await relayAuth.headers(config),
        nsec: key.nsec,
        kind: 30177,
        content: jsonEncode({
          'name': setup['name'],
          'picture': ?setup['picture'],
          'parallelism': 1,
          'respond_to': 'owner-only',
        }),
        tags: [
          ['d', pubkey],
          ['runtime', 'block.builderlab/builderlab'],
        ],
      );
    });
  }

  /// Removes only this owner's registration in [relay].
  Future<void> removePolicy(String relay, String pubkey) =>
      keys.withKeys((key) async {
        final config = RelayConfig(baseUrl: relay, nsec: key.nsec);
        await submitSignedEventOnce(
          wsUrl: config.wsUrl,
          headers: await relayAuth.headers(config),
          nsec: key.nsec,
          kind: 5,
          content: '',
          tags: [
            ['a', '30177:${key.public}:$pubkey'],
          ],
        );
      });

  /// Deletes an owned runtime only after its registration in [community] is
  /// deleted and a fresh relay read confirms its absence.
  Future<bool> deleteAgent({
    required String pubkey,
    required Community community,
    required bool Function() isCurrent,
    required Future<List<NostrEvent>> Function(NostrFilter) fetch,
  }) => run(() async {
    final subject = account.account?.subject;
    void check() {
      if (!isCurrent() || account.account?.subject != subject) {
        throw StateError('Community or account changed. Reopen Agents.');
      }
    }

    if (!HostedEnvironment.supportsRelay(Uri.parse(community.relayUrl))) {
      throw StateError('This community does not support remote agents.');
    }
    check();
    await ensureBinding();
    check();
    final scope = _storageScope;
    // Agents belong to the member who owns them (NIP-OA), not the community.
    final member = community.pubkey;
    if (member == null || owner != member) {
      throw AccountIdentityFailure.mismatch;
    }
    final agent = agents
        .where((row) => row['agent_pubkey'] == pubkey)
        .firstOrNull;
    final id = agent?['agent_id'];
    if (agent == null || id is! String || id.isEmpty) {
      throw StateError('Owned remote agent ID is unavailable. Refresh Agents.');
    }
    // Block only the setup's own agent, or any agent while the setup's
    // identity is unknown, since it could be the one being deleted.
    final setupAgent = pending?['pubkey'];
    if (setupAgent == pubkey ||
        (setupAgent == null && pending?['registering'] == true)) {
      throw StateError(
        'Resume the unfinished setup before deleting this agent.',
      );
    }
    final prefs = await SharedPreferences.getInstance();
    check();
    final tombstones = prefs.getStringList('$scope:deleting')?.toSet() ?? {};
    tombstones.add(pubkey);
    if (!await prefs.setStringList('$scope:deleting', tombstones.toList())) {
      throw StateError('Could not save deletion intent. Retry Delete.');
    }
    final retries = Map<String, dynamic>.from(
      jsonDecode(prefs.getString('$scope:delete-retries') ?? '{}') as Map,
    );
    retries[pubkey] = agent;
    if (!await prefs.setString('$scope:delete-retries', jsonEncode(retries))) {
      throw StateError('Could not save deletion retry. Retry Delete.');
    }
    check();
    await removePolicy(community.relayUrl, pubkey);
    _registrationsChanged();
    check();
    final remaining = await fetch(
      NostrFilter(
        kinds: const [30177],
        authors: [member],
        tags: {
          '#d': [pubkey],
        },
      ),
    );
    check();
    if (remaining.isNotEmpty) {
      throw StateError('Community registration still exists. Retry Delete.');
    }
    final result = await account.post('/v3/beekeeper/delete-agent', {
      'agent_id': id,
    });
    check();
    final status = result['status'];
    if (status != 1 &&
        status != 3 &&
        status != 'DELETE_AGENT_STATUS_DELETED' &&
        status != 'DELETE_AGENT_STATUS_NOT_FOUND') {
      throw StateError(
        status == 2 || status == 'DELETE_AGENT_STATUS_DISABLED'
            ? 'Agent deletion is disabled on this server.'
            : 'Agent deletion was not confirmed. Retry Delete.',
      );
    }
    final completed = {...deletedAgents, pubkey};
    if (!await prefs.setStringList('$scope:deleted', completed.toList())) {
      throw StateError(
        'Runtime deleted, but local cleanup failed. Retry Delete.',
      );
    }
    deletedAgents = completed;
    retries.remove(pubkey);
    if (!await prefs.setString('$scope:delete-retries', jsonEncode(retries))) {
      throw StateError(
        'Runtime deleted, but local cleanup failed. Retry Delete.',
      );
    }
    agents = agents.where((row) => row['agent_pubkey'] != pubkey).toList();
  });

  @override
  void dispose() {
    _disposed = true;
    account.subjectChanges.removeListener(_accountChanged);
    super.dispose();
  }
}

/// Signs the NIP-OA protocol preimage, not a Nostr event ID.
List<String> signOwnerAuthorization(nostr.Keys owner, String agent) {
  if (!RegExp(r'^[0-9a-f]{64}$').hasMatch(agent) || agent == owner.public) {
    throw StateError('Invalid agent key.');
  }
  final digest = sha256
      .convert(utf8.encode('nostr:agent-auth:$agent:'))
      .toString();
  return [
    'auth',
    owner.public,
    '',
    signSchnorrHex(secretKey: owner.secret, messageHex: digest),
  ];
}

/// Accepts the registry's numeric or `*_<suffix>` enum form, nothing else.
void requireStatus(Map<String, dynamic> result, String suffix, int numeric) {
  if (hasStatus(result, suffix, numeric)) return;
  final status = result['status'];
  throw StateError(
    'Builderlab did not confirm $suffix '
    '(${status is String || status is num ? status : 'missing status'}).',
  );
}

/// Whether [result] reports [suffix] in the registry's numeric or `*_<suffix>`
/// enum form.
bool hasStatus(Map<String, dynamic> result, String suffix, int numeric) {
  final status = result['status'];
  return status == numeric || (status is String && status.endsWith('_$suffix'));
}

/// A service-provided model and its supported reasoning effort values.
class RemoteAgentModel {
  const RemoteAgentModel({
    required this.id,
    required this.name,
    required this.efforts,
  });
  final String id;
  final String name;
  final List<String> efforts;

  factory RemoteAgentModel.fromJson(dynamic value) {
    if (value is! Map ||
        value['id'] is! String ||
        (value['id'] as String).trim().isEmpty ||
        value['display_name'] is! String ||
        (value['display_name'] as String).trim().isEmpty ||
        value['supported_reasoning_efforts'] is! List) {
      throw StateError('Invalid model catalog.');
    }
    final efforts = value['supported_reasoning_efforts'] as List;
    if (efforts.isEmpty ||
        efforts.any((item) => item is! String || item.trim().isEmpty)) {
      throw StateError('Invalid model catalog.');
    }
    return RemoteAgentModel(
      id: value['id'] as String,
      name: value['display_name'] as String,
      efforts: List<String>.from(efforts),
    );
  }
}
