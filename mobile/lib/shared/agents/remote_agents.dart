import 'dart:async';
import 'dart:convert';

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
  void changed() {
    if (!setEquals(controller.activeAgents, active)) ref.invalidateSelf();
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
          verifiedOaOwnerPubkey(event.tags, event.pubkey) == owner)
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
  String status = '';
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

  /// Runs one operation at a time and turns its failure into [status].
  Future<bool> run(Future<void> Function() action) async {
    if (busy || _disposed) return false;
    busy = true;
    final operation = ++_operation;
    notifyListeners();
    try {
      await action();
      return true;
    } catch (error) {
      // Account failures never carry credentials or response bodies.
      status = switch (error) {
        AccountFailure.signedOut => 'Sign in to Builderlab again.',
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

  Future<void> refresh() async {
    final session = account.account ?? await account.restore();
    if (session == null) {
      _clearAccountState();
      status = 'Sign in to Builderlab to manage agents.';
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
    status = switch (pending) {
      null => '',
      {'instructionsMissing': true} =>
        'Unfinished setup saved. Enter its instructions again, then Resume.',
      _ => 'Unfinished setup saved. Resume to use the same agent.',
    };
  }

  /// Requires this account's saved key to be its current server binding.
  Future<void> ensureBinding() async {
    final subject = account.account?.subject;
    if (subject == null) throw AccountFailure.signedOut;
    if (_scope != _storageScope) _clearAccountState();
    final local = await keys.read();
    if (local == null) throw AccountIdentityFailure.absent;
    final response = await account.currentIdentity();
    if (account.account?.subject != subject) {
      throw AccountFailure.differentAccount;
    }
    if (response['error'] != null) throw AccountFailure.invalidResponse;
    final bound = (response['identity'] as Map?)?['pubkey_hex'];
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

  /// Registers, activates and subscribes an agent in [community]. A failed
  /// attempt keeps its identity so the next call resumes the same agent.
  Future<bool> create(
    Community community,
    String name, {
    String instructions = '',
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
      await _savePending({
        'name': trimmed,
        'instructions': instructions.trim(),
        'community': community.relayUrl,
        'owner': owner,
        'request': const Uuid().v4(),
      });
    }
    var setup = Map<String, dynamic>.from(pending!);
    if (setup['owner'] != owner || setup['community'] != community.relayUrl) {
      throw StateError('Finish the pending setup in its original community.');
    }
    if (setup['instructionsMissing'] == true) {
      if (instructions.trim().length > 20000) {
        throw StateError('Use at most 20,000 characters for instructions.');
      }
      setup = {...setup, 'instructions': instructions.trim()}
        ..remove('instructionsMissing');
      await _savePending(setup);
    }
    if (setup['pubkey'] == null) {
      status = 'Creating remote identity…';
      _notify();
      final result = await account.post('/v3/beekeeper/register-agent', {
        'agent_name': setup['name'],
        'idempotency_key': setup['request'],
      });
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
    final pubkey = setup['pubkey'] as String;
    status = 'Authorizing and activating agent…';
    _notify();
    final auth = await keys.withKeys(
      (key) async => signOwnerAuthorization(key, pubkey),
    );
    final result = await account.post('/v3/beekeeper/attest-agent', {
      'agent_pubkey': pubkey,
      'owner_auth_tag_json': jsonEncode(auth),
      'community_url': community.relayUrl,
    });
    requireStatus(result, 'ACTIVE', 1);
    if ((setup['instructions'] as String?)?.isNotEmpty == true) {
      await account.post('/v3/beekeeper/update-agent', {
        'agent_pubkey': pubkey,
        'agent_instructions': setup['instructions'],
      });
    }
    await subscribe(setup['id'] as String, community.relayUrl);
    await publishPolicy(community.relayUrl, setup);
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
    status = '${setup['name']} is ready. Mention it in a channel.';
  });

  /// Abandons an unfinished setup that never got a server identity, such as
  /// one whose registration the server rejects. Setups with an identity can
  /// only be resumed, so no registered agent is orphaned.
  Future<bool> discardPending() => run(() async {
    final setup = pending;
    if (setup == null) return;
    if (setup['pubkey'] != null) {
      throw StateError('This setup already has an agent. Resume it.');
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
    status = '';
  });

  /// Activation alone only reconciles server-configured communities, so a
  /// community the owner joined later needs an explicit subscription.
  Future<void> subscribe(String agentId, String relay) async {
    final result = await account.post('/v3/beekeeper/add-buzz-subscription', {
      'agent_id': agentId,
      'community': Uri.parse(relay).host,
    });
    final value = result['status'];
    if (value != 1 &&
        value != 2 &&
        value != 'ADD_BUZZ_SUBSCRIPTION_STATUS_CREATED' &&
        value != 'ADD_BUZZ_SUBSCRIPTION_STATUS_ALREADY_EXISTS') {
      throw StateError('Community subscription was not confirmed ($value).');
    }
  }

  /// Publishes the owner's agent registration only after activation and
  /// subscription succeed.
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
    if (pending != null) {
      throw StateError('Finish pending agent setup before deleting an agent.');
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
    this.status = 'Remote agent deleted.';
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
    nostr.Schnorr.sign(secretKey: owner.secret, message: digest),
  ];
}

/// Accepts the registry's numeric or `*_<suffix>` enum form, nothing else.
void requireStatus(Map<String, dynamic> result, String suffix, int numeric) {
  final status = result['status'];
  if (status == numeric || (status is String && status.endsWith('_$suffix'))) {
    return;
  }
  throw StateError(
    'Builderlab did not confirm $suffix '
    '(${status is String || status is num ? status : 'missing status'}).',
  );
}
