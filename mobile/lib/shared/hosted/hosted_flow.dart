import '../storage/serial_queue.dart';
import 'dart:async';

import 'package:flutter/foundation.dart';
import 'package:url_launcher/url_launcher.dart';

import '../deeplink/deep_link.dart';
import '../community/community.dart';
import 'account_identity_store.dart';
import 'device_identity_source.dart';
import 'browser_auth.dart';
import 'builderlab_account.dart';
import 'hosted_identity_service.dart';
import 'hosted_backup_service.dart';
import 'hpke_backup.dart';
import 'hosted_environment.dart';
import 'hosted_communities.dart';
import 'hosted_community_creation.dart';
import 'hosted_community_connection.dart';
import 'web_connect_binding.dart';
import 'hosted_join_policy.dart';

enum HostedFlowStage {
  idle,
  working,
  signIn,
  chooseIdentity,
  chooseCommunities,
  communityConflict,
  acceptPolicy,
  profile,
  blocked,
  browserReturned,
  complete,
}

enum HostedFlowProblem {
  retry,
  denied,
  accountSetupRequired,
  differentAccount,
  identityUnreadable,
  backupUnavailable,
  invalidResponse,
  requestRejected,
  identityMismatch,
  identityUnavailable,
  backupRequired,
  bindingRequired,
  unsupportedEnvironment,
  invitePolicyRequired,
  inviteExpired,
  inviteExhausted,
  inviteInvalid,
  communityCreationRejected,
}

/// Resolves only after relay admission is proven and local persistence completes.
/// Implementations must check [isCurrent] immediately before durable mutations.
typedef HostedCommunityAdmission =
    Future<void> Function(
      BuzzDeepLink target,
      String subject,
      String pubkey,
      bool Function() isCurrent,
    );

/// App-scoped owner: callback subscriptions must outlive any individual page.
/// Only public progress reaches widgets; pending auth secrets remain in BrowserAuth.
class HostedFlow extends ChangeNotifier {
  HostedFlow({
    required this.account,
    required this.browser,
    required this.store,
    required this.identity,
    required this.enterCommunity,
    Future<bool> Function(Uri)? openWeb,
  }) : _openWeb =
           openWeb ??
           ((uri) => launchUrl(uri, mode: LaunchMode.externalApplication));
  final BuilderlabAccountClient account;
  final BrowserAuth browser;
  final AccountIdentityStore store;
  final HostedIdentityService identity;
  final HostedCommunityAdmission enterCommunity;
  bool get canRetry =>
      problem == null ||
      problem == HostedFlowProblem.retry ||
      problem == HostedFlowProblem.invitePolicyRequired;
  final Future<bool> Function(Uri) _openWeb;
  HostedFlowStage stage = HostedFlowStage.idle;
  HostedFlowProblem? problem;
  List<DeviceIdentityChoice> identityChoices = const [];
  BuzzDeepLink? _target;
  bool accountOnly = false;
  String? _pairedPubkey;
  SensitiveActionPolicy? pairedSensitiveActionPolicy;
  List<AddCommunityDeepLink> communities = const [];
  Set<String> selectedRelays = const {};
  final Set<String> _addedRelays = {};
  bool _addingCommunities = false;
  String? _batchSubject;
  String? _conflictingRelay;
  bool get canChangeCommunities =>
      accountOnly &&
      account.account?.subject == _batchSubject &&
      (stage == HostedFlowStage.blocked ||
          stage == HostedFlowStage.communityConflict) &&
      communities.isNotEmpty;
  String? addingCommunityName;
  int get addedCommunityCount => _addedRelays.length;
  final _queue = SerialQueue();
  StreamSubscription<BrowserAuthCode>? _codes;
  StreamSubscription<BrowserAuthFailure>? _failures;
  bool _disposed = false;
  int _generation = 0;

  int _nextGeneration() {
    _cancelPolicy();
    _cancelProfile();
    return ++_generation;
  }

  Completer<void>? _profileCompletion;
  Future<void> Function(String)? saveProfile;

  Future<void> Function(String, Future<String> Function()?)? saveProfileDraft;

  Future<void> requestProfile(
    Future<void> Function(String) save, {
    Future<void> Function(String, Future<String> Function()?)? saveDraft,
  }) async {
    if (_disposed) return;
    final completion = Completer<void>();
    _profileCompletion = completion;
    saveProfileDraft = (name, avatar) async {
      if (!identical(_profileCompletion, completion) ||
          completion.isCompleted) {
        throw AccountFailure.unavailable;
      }
      if (saveDraft != null) {
        await saveDraft(name, avatar);
      } else {
        await save(name);
      }
      if (identical(_profileCompletion, completion)) finishProfile();
    };
    final saveCurrentDraft = saveProfileDraft!;
    saveProfile = (name) => saveCurrentDraft(name, null);
    _set(HostedFlowStage.profile);
    await completion.future;
    if (identical(_profileCompletion, completion)) {
      _profileCompletion = null;
      saveProfile = null;
      saveProfileDraft = null;
      _set(HostedFlowStage.working);
    }
  }

  void finishProfile() {
    final completion = _profileCompletion;
    if (completion != null && !completion.isCompleted) completion.complete();
  }

  void _cancelProfile() {
    final completion = _profileCompletion;
    _profileCompletion = null;
    saveProfile = null;
    saveProfileDraft = null;
    if (completion != null && !completion.isCompleted) completion.complete();
  }

  HostedJoinPolicy? joinPolicy;
  Completer<bool>? _policyConsent;

  Future<bool> confirmJoinPolicy(HostedJoinPolicy policy) async {
    if (_disposed) return false;
    joinPolicy = policy;
    final consent = Completer<bool>();
    _policyConsent = consent;
    _set(HostedFlowStage.acceptPolicy);
    final accepted = await consent.future;
    if (identical(_policyConsent, consent)) {
      _policyConsent = null;
      joinPolicy = null;
      if (accepted) _set(HostedFlowStage.working);
    }
    return accepted;
  }

  void acceptJoinPolicy() {
    final consent = _policyConsent;
    if (consent != null && !consent.isCompleted) consent.complete(true);
  }

  void _cancelPolicy() {
    final consent = _policyConsent;
    _policyConsent = null;
    joinPolicy = null;
    if (consent != null && !consent.isCompleted) consent.complete(false);
  }

  bool reconnecting = false;

  Future<void>? _initialization;
  Future<void> initialize() => _initialization ??= _initialize();

  Future<void> _initialize() async {
    if (_codes != null) return;
    _codes = browser.codes.listen((code) {
      final generation = _generation;
      _enqueue(() async {
        reconnecting = code.intent['type'] == 'reconnect';
        accountOnly = reconnecting || code.intent['type'] == 'account';
        if (accountOnly) {
          _target = null;
        } else {
          _restorePairing(code.intent);
          _target = _decode(code.intent);
          if (_target == null) throw BrowserAuthFailure.restartLogin;
        }
        _set(HostedFlowStage.working);
        await account.exchange(code: code.code, verifier: code.verifier);
        if (_disposed || generation != _generation) return;
        if (reconnecting) {
          await browser.clearIntent();
          if (_disposed || generation != _generation) return;
          _set(HostedFlowStage.complete);
          reconnecting = false;
        } else {
          await _continue(generation);
        }
      });
    });
    _failures = browser.failures.listen((failure) {
      if (_disposed) return;
      if (!accountOnly && _target == null) {
        if (_generation == 0) {
          _set(HostedFlowStage.blocked, HostedFlowProblem.retry);
        }
        return;
      }
      if (stage != HostedFlowStage.signIn) return;
      if (failure == BrowserAuthFailure.cancelled) {
        unawaited(leave());
      } else {
        _set(HostedFlowStage.blocked, HostedFlowProblem.retry);
      }
    });
    await _enqueue(() async {
      final generation = _generation;
      await browser.initialize();
      if (_disposed || generation != _generation) return;
      if (_target == null) {
        final retained = await browser.retainedIntent();
        if (_disposed || generation != _generation) return;
        if (retained != null && _target == null) {
          if (retained['type'] == 'reconnect') {
            reconnecting = true;
            accountOnly = true;
            _set(HostedFlowStage.idle);
          } else if (retained['type'] == 'account') {
            accountOnly = true;
            _set(HostedFlowStage.idle);
          } else {
            _restorePairing(retained);
            _target = _decode(retained);
          }
          if (_target != null && stage == HostedFlowStage.idle) {
            _set(HostedFlowStage.signIn);
          }
        }
      }
    });
  }

  /// Reauthenticate an existing community without enrollment, backup or discovery.
  Future<void> reconnectAccount({bool changeAccount = false}) async {
    await initialize();
    final generation = _nextGeneration();
    reconnecting = true;
    accountOnly = true;
    _target = null;
    _set(HostedFlowStage.signIn);
    await _enqueue(() async {
      if (_disposed || generation != _generation) return;
      if (changeAccount) await account.clearLocalSession();
      if (_disposed || generation != _generation) return;
      await browser.start({'type': 'reconnect'});
    });
  }

  /// Explicit Connect completes identity setup and discovers joined communities.
  /// Passive startup restore only checks the saved session.
  Future<void> signInAccount({bool openBrowser = true}) {
    if (_target != null ||
        stage == HostedFlowStage.working ||
        stage == HostedFlowStage.signIn) {
      return Future.value();
    }
    final generation = openBrowser ? _nextGeneration() : _generation;
    if (openBrowser) {
      reconnecting = false;
      accountOnly = true;
      _clearCommunityBatch();
      _set(HostedFlowStage.working);
    }
    return _enqueue(() async {
      if (!openBrowser) {
        if (_target != null || stage != HostedFlowStage.idle) return;
      }
      final restored = await account.restore();
      if (_disposed || generation != _generation) return;
      if (!openBrowser) {
        // Mounting the welcome page must not create/bind/back up a key.
        _set(HostedFlowStage.idle);
      } else if (restored != null) {
        await _continue(generation);
      } else {
        _set(HostedFlowStage.signIn);
        await browser.start({'type': 'account'});
      }
    });
  }

  /// Securely retain the transferred identity before NIP-AB completion. Hosted
  /// admission still waits for account, binding and backup in the normal flow.
  Future<void> stagePairedCommunity(
    Community community, {
    required bool Function() isCurrent,
  }) async {
    final uri = Uri.parse(community.relayUrl);
    if (!HostedEnvironment.supportsRelay(uri)) throw AccountFailure.denied;
    final nsec = community.nsec;
    if (nsec == null) throw AccountIdentityFailure.absent;
    final pubkey = AccountIdentityStore.publicKeyForNsec(nsec);
    if (community.pubkey != null && community.pubkey != pubkey) {
      throw AccountIdentityFailure.mismatch;
    }
    final device = await DeviceIdentitySource().choices();
    if (device.isNotEmpty && !device.any((item) => item.pubkey == pubkey)) {
      throw AccountIdentityFailure.mismatch;
    }
    final generation = _nextGeneration();
    void check() {
      if (!isCurrent() || generation != _generation || _disposed) {
        throw AccountFailure.unavailable;
      }
    }

    check();
    await store.forPairing(pubkey).create(nsec, checkCurrent: check);
    check();
    final relay = uri.replace(scheme: 'wss').toString();
    final target = AddCommunityDeepLink(relayUrl: relay, name: community.name);
    _pairedPubkey = pubkey;
    pairedSensitiveActionPolicy = community.sensitiveActionPolicy;
    await browser.retainIntent(_intent(target));
    check();
    _target = target;
    accountOnly = false;
    _set(HostedFlowStage.signIn);
  }

  void _restorePairing(Map<String, String> intent) {
    _pairedPubkey = intent['paired_pubkey'];
    pairedSensitiveActionPolicy = intent['paired_protection'] == 'enabled'
        ? SensitiveActionPolicy.enabled
        : null;
  }

  Map<String, String> _intent(BuzzDeepLink target) => {
    ..._encode(target),
    'paired_pubkey': ?_pairedPubkey,
    if (pairedSensitiveActionPolicy == SensitiveActionPolicy.enabled)
      'paired_protection': 'enabled',
  };

  Future<void> start(BuzzDeepLink target, {bool resumePairing = false}) {
    if (!resumePairing) {
      _pairedPubkey = null;
      pairedSensitiveActionPolicy = null;
    }
    if (target is! NostrBindDeepLink &&
        target is! AddCommunityDeepLink &&
        target is! InviteDeepLink) {
      throw ArgumentError('Unsupported hosted target');
    }
    final generation = _nextGeneration();
    reconnecting = false;
    accountOnly = false;
    _clearCommunityBatch();
    _target = target;
    _set(HostedFlowStage.working);
    return _enqueue(() async {
      if (generation != _generation) return;
      if (!_acceptTarget(target)) return;
      // Login may already be complete. Retain the destination before any
      // remote work so restart can reconcile using the same saved identity.
      await browser.retainIntent(_intent(target));
      if (_disposed || generation != _generation) return;
      await _resume(generation, _intent(target));
    });
  }

  Future<void> _resume(int generation, Map<String, String> intent) async {
    final restored = await account.restore();
    if (_disposed || generation != _generation) return;
    if (restored == null) {
      _set(HostedFlowStage.signIn);
      await browser.start(intent);
    } else {
      await _continue(generation);
    }
  }

  Future<void> retry() async {
    if (reconnecting) return reconnectAccount();
    final target = _target;
    if (target == null && !accountOnly) return;
    final generation = _nextGeneration();
    try {
      await browser.cancel();
      if (_disposed || generation != _generation) return;
      if (target != null) {
        await start(target, resumePairing: _pairedPubkey != null);
      } else {
        _set(HostedFlowStage.working);
        await _enqueue(() => _resume(generation, {'type': 'account'}));
      }
    } catch (_) {
      if (!_disposed && generation == _generation) {
        _set(HostedFlowStage.blocked, HostedFlowProblem.retry);
      }
    }
  }

  /// Resume only an offered public identity; reread local and cloud facts first.
  Future<void> chooseIdentity(String pubkey) {
    if ((_target == null && !accountOnly) ||
        stage != HostedFlowStage.chooseIdentity ||
        !identityChoices.any((item) => item.pubkey == pubkey)) {
      return Future.value();
    }
    final generation = _nextGeneration();
    _set(HostedFlowStage.working);
    return _enqueue(() => _continue(generation, selectedPubkey: pubkey));
  }

  Future<void> _continue(int generation, {String? selectedPubkey}) async {
    final target = _target;
    final session = account.account;
    if ((target == null && !accountOnly) || session == null) {
      throw AccountFailure.signedOut;
    }
    final store = this.store.forSubject(session.subject);
    if (target != null && !_acceptTarget(target)) return;
    if (target == null && _batchSubject != session.subject) {
      _clearCommunityBatch();
      _batchSubject = session.subject;
    }
    var resolved = await identity.resolve(
      isCurrent: _isCurrent(generation),
      selectedPubkey: selectedPubkey ?? _pairedPubkey,
    );
    if (_pairedPubkey != null && resolved.pubkey != _pairedPubkey) {
      _set(HostedFlowStage.blocked, HostedFlowProblem.identityMismatch);
      return;
    }
    if (target is! NostrBindDeepLink) {
      for (var step = 0; step < 2; step++) {
        if (_disposed || generation != _generation) return;
        if (resolved.state == HostedIdentityState.needsBinding &&
            resolved.pubkey != null) {
          final response = await account.post(
            '/v1/buzz/nostr-identities/challenge',
            {'origin': HostedEnvironment.origin.toString()},
          );
          if (_disposed || generation != _generation) return;
          final fields = <String, String>{
            'audience': 'buzz:nostr-identity',
            'action': 'bind_nostr_identity',
            'protocol': 'buzz-nostr-identity',
            'version': '1',
            'return': 'browser_fragment_v1',
            'callback_url': HostedEnvironment.origin
                .resolve('/buzz')
                .toString(),
          };
          for (final key in [
            'challenge_id',
            'nonce',
            'verification_code',
            'origin',
            'expires_at',
          ]) {
            final value = response[key];
            if (value is! String) throw AccountFailure.invalidResponse;
            fields[key] = value;
          }
          final request = parseNostrBindDeepLink(
            Uri(scheme: 'buzz', host: 'nostr-bind', queryParameters: fields),
          );
          if (response['error'] != null || request == null) {
            throw AccountFailure.invalidResponse;
          }
          await WebConnectBindingService().complete(
            request: request,
            subject: session.subject,
            expectedPubkey: resolved.pubkey!,
            withKeys: store.withKeys,
            verify: _verifyBinding,
            isCurrent: _isCurrent(generation),
          );
        } else if (resolved.state == HostedIdentityState.needsBackup) {
          await HostedBackupService(
            account,
            store,
          ).backup(isCurrent: _isCurrent(generation));
        } else {
          break;
        }
        if (_disposed || generation != _generation) return;
        resolved = await identity.resolve(isCurrent: _isCurrent(generation));
      }
    }
    if (generation != _generation || _disposed) return;
    if (resolved.state == HostedIdentityState.needsSelection) {
      identityChoices = List.unmodifiable(resolved.choices);
      _set(HostedFlowStage.chooseIdentity);
      return;
    }
    final pubkey = resolved.pubkey;
    if (target is NostrBindDeepLink &&
        pubkey != null &&
        {
          HostedIdentityState.ready,
          HostedIdentityState.needsBinding,
          HostedIdentityState.needsBackup,
        }.contains(resolved.state)) {
      final returnTo = await WebConnectBindingService().complete(
        request: target,
        subject: session.subject,
        expectedPubkey: pubkey,
        withKeys: store.withKeys,
        verify: _verifyBinding,
        isCurrent: _isCurrent(generation),
      );
      if (generation != _generation || _disposed) return;
      if (!await _openWeb(returnTo)) throw BrowserAuthFailure.unavailable;
      await browser.clearIntent();
      _set(HostedFlowStage.browserReturned);
      return;
    }
    if (resolved.state != HostedIdentityState.ready || pubkey == null) {
      final failure = switch (resolved.state) {
        HostedIdentityState.needsBackup => HostedFlowProblem.backupRequired,
        HostedIdentityState.needsBinding => HostedFlowProblem.bindingRequired,
        HostedIdentityState.mismatch => HostedFlowProblem.identityMismatch,
        _ => HostedFlowProblem.identityUnavailable,
      };
      _set(HostedFlowStage.blocked, failure);
      return;
    }
    if (target == null) {
      await _continueCommunities(generation, session.subject, pubkey);
      return;
    }
    final destination = target is HostedCreateCommunity
        ? await target.provision(account, pubkey, _isCurrent(generation))
        : target;
    if (_disposed || generation != _generation) return;
    await enterCommunity(
      destination,
      session.subject,
      pubkey,
      _isCurrent(generation),
    );
    if (generation != _generation || _disposed) return;
    await browser.clearIntent();
    _set(HostedFlowStage.complete);
  }

  void selectCommunity(String relay, bool selected) {
    if (stage != HostedFlowStage.chooseCommunities ||
        !communities.any((item) => item.relayUrl == relay)) {
      return;
    }
    selectedRelays = Set.unmodifiable({
      ...selectedRelays.where((item) => item != relay),
      if (selected) relay,
    });
    notifyListeners();
  }

  Future<void> addSelectedCommunities() {
    if (!accountOnly ||
        stage != HostedFlowStage.chooseCommunities ||
        selectedRelays.isEmpty) {
      return Future.value();
    }
    _addingCommunities = true;
    final generation = _nextGeneration();
    _set(HostedFlowStage.working);
    return _enqueue(() => _continue(generation));
  }

  /// Keep completed additions and let the user omit a failing target.
  void backToCommunityChoices() {
    if (!canChangeCommunities) return;
    _nextGeneration();
    communities = List.unmodifiable(
      communities.where((item) => !_addedRelays.contains(item.relayUrl)),
    );
    selectedRelays = Set.unmodifiable(selectedRelays.difference(_addedRelays));
    _conflictingRelay = null;
    addingCommunityName = null;
    _set(HostedFlowStage.chooseCommunities);
  }

  Future<void> skipConflictingCommunity() {
    final relay = _conflictingRelay;
    if (stage != HostedFlowStage.communityConflict || relay == null) {
      return Future.value();
    }
    selectedRelays = Set.unmodifiable(
      selectedRelays.where((item) => item != relay),
    );
    _conflictingRelay = null;
    final generation = _nextGeneration();
    _set(HostedFlowStage.working);
    return _enqueue(() => _continue(generation));
  }

  Future<void> _continueCommunities(
    int generation,
    String subject,
    String pubkey,
  ) async {
    bool current() =>
        !_disposed &&
        generation == _generation &&
        account.account?.subject == subject;
    if (!_addingCommunities) {
      final listed = await loadHostedCommunities(account);
      if (!current()) return;
      communities = listed;
      selectedRelays = Set.unmodifiable(listed.map((item) => item.relayUrl));
      if (listed.isEmpty) {
        await leave(noCommunities: true);
        return;
      }
      // Direct account entry imports every joined community. The existing
      // switcher handles choosing which community to enter after admission.
      _addingCommunities = true;
    }
    for (final community in communities) {
      if (!current()) return;
      if (!selectedRelays.contains(community.relayUrl) ||
          _addedRelays.contains(community.relayUrl)) {
        continue;
      }
      addingCommunityName = community.name;
      notifyListeners();
      try {
        await enterCommunity(community, subject, pubkey, current);
      } on SavedCommunityIdentityConflict {
        if (!current()) return;
        _conflictingRelay = community.relayUrl;
        _set(HostedFlowStage.communityConflict);
        return;
      }
      if (!current()) return;
      _addedRelays.add(community.relayUrl);
    }
    if (!current()) return;
    addingCommunityName = null;
    if (_addedRelays.isEmpty) {
      communities = List.unmodifiable(
        communities.where((item) => selectedRelays.contains(item.relayUrl)),
      );
      await leave(noCommunities: true);
      return;
    }
    await browser.clearIntent();
    if (current()) _set(HostedFlowStage.complete);
  }

  bool _acceptTarget(BuzzDeepLink target) {
    final relay = target.relayUrl;
    if (relay != null && !HostedEnvironment.supportsRelay(Uri.parse(relay))) {
      _set(HostedFlowStage.blocked, HostedFlowProblem.unsupportedEnvironment);
      return false;
    }
    return true;
  }

  Future<String> _verifyBinding({
    required String subject,
    required String challengeId,
    required String nonce,
    required String signedPayload,
  }) async {
    if (account.account?.subject != subject) {
      throw AccountFailure.differentAccount;
    }
    final response = await account.post('/v1/buzz/nostr-identities/verify', {
      'challenge_id': challengeId,
      'nonce': nonce,
      'signed_payload': signedPayload,
    });
    if (account.account?.subject != subject) {
      throw AccountFailure.differentAccount;
    }
    final bound = response['identity'];
    if (response['error'] != null ||
        bound is! Map ||
        bound['pubkey_hex'] is! String) {
      throw AccountFailure.invalidResponse;
    }
    return bound['pubkey_hex'] as String;
  }

  bool Function() _isCurrent(int generation) =>
      () => !_disposed && generation == _generation;

  Future<void> _enqueue(Future<void> Function() operation) {
    final generation = _generation;
    final result = _queue.run(() async {
      if (_disposed || generation != _generation) return;
      try {
        await operation();
      } catch (error) {
        if (!_disposed && generation == _generation) {
          _set(HostedFlowStage.blocked, _problemFor(error));
        }
      }
    });
    return result;
  }

  static HostedFlowProblem _problemFor(Object error) => switch (error) {
    HostedCreationFailure.rejected =>
      HostedFlowProblem.communityCreationRejected,
    HostedInviteFailure.policyRequired =>
      HostedFlowProblem.invitePolicyRequired,
    HostedInviteFailure.expired => HostedFlowProblem.inviteExpired,
    HostedInviteFailure.exhausted => HostedFlowProblem.inviteExhausted,
    HostedInviteFailure.invalid => HostedFlowProblem.inviteInvalid,
    AccountFailure.denied => HostedFlowProblem.denied,
    AccountFailure.accountSetupRequired =>
      HostedFlowProblem.accountSetupRequired,
    AccountFailure.rejected ||
    HostedBackupFailure.rejected => HostedFlowProblem.requestRejected,
    HostedBackupFailure.unavailable => HostedFlowProblem.backupUnavailable,
    AccountFailure.differentAccount => HostedFlowProblem.differentAccount,
    AccountIdentityFailure.unreadable ||
    AccountIdentityFailure.invalidKey ||
    AccountFailure.storage ||
    HostedBackupFailure.storage => HostedFlowProblem.identityUnreadable,
    SavedCommunityIdentityConflict() ||
    AccountIdentityFailure.mismatch ||
    AccountIdentityFailure.occupied ||
    HostedBackupFailure.identityMismatch ||
    HpkeBackupFailure.identityMismatch => HostedFlowProblem.identityMismatch,
    HpkeBackupFailure.unavailable => HostedFlowProblem.backupUnavailable,
    AccountFailure.invalidResponse ||
    HostedBackupFailure.invalidResponse ||
    HpkeBackupFailure.invalidEnrollment => HostedFlowProblem.invalidResponse,
    _ => HostedFlowProblem.retry,
  };

  void _set(HostedFlowStage next, [HostedFlowProblem? failure]) {
    if (_disposed) return;
    if (next != HostedFlowStage.chooseIdentity) identityChoices = const [];
    stage = next;
    problem = failure;
    notifyListeners();
  }

  bool noCommunitiesToAdd = false;

  void _clearCommunityBatch() {
    noCommunitiesToAdd = false;
    communities = const [];
    selectedRelays = const {};
    _addedRelays.clear();
    _batchSubject = null;
    _conflictingRelay = null;
    _addingCommunities = false;
    addingCommunityName = null;
  }

  Future<void> leave({bool noCommunities = false}) async {
    _nextGeneration();
    final generation = _generation;
    _target = null;
    _pairedPubkey = null;
    pairedSensitiveActionPolicy = null;
    reconnecting = false;
    accountOnly = false;
    _clearCommunityBatch();
    noCommunitiesToAdd = noCommunities;
    try {
      await browser.cancel();
      if (_disposed || generation != _generation) return;
      await browser.clearIntent();
      if (generation == _generation) _set(HostedFlowStage.idle);
    } catch (_) {
      if (generation == _generation) {
        _set(HostedFlowStage.blocked, HostedFlowProblem.retry);
      }
    }
  }

  static Map<String, String> _encode(BuzzDeepLink target) {
    if (target is HostedCreateCommunity) {
      return {'type': 'create', 'name': target.name};
    }
    if (target is NostrBindDeepLink) return {'type': 'bind', ...target.fields};
    if (target is AddCommunityDeepLink) {
      return {'type': 'add', 'relay': target.relayUrl, 'name': target.name};
    }
    final invite = target as InviteDeepLink;
    return {
      'type': 'invite',
      'relay': invite.relayUrl,
      'code': invite.code,
      if (invite.policyReceipt != null) 'policy_receipt': invite.policyReceipt!,
    };
  }

  static BuzzDeepLink? _decode(Map<String, String> intent) {
    if (intent['type'] == 'create') {
      final name = intent['name'];
      return name != null && HostedCreateCommunity.validName(name)
          ? HostedCreateCommunity(name)
          : null;
    }
    final fields = Map<String, String>.of(intent)
      ..remove('type')
      ..remove('paired_pubkey')
      ..remove('paired_protection');
    final host = switch (intent['type']) {
      'bind' => 'nostr-bind',
      'add' => 'add-community',
      'invite' => 'join',
      _ => '',
    };
    return parseBuzzDeepLink(
      Uri(scheme: 'buzz', host: host, queryParameters: fields),
    );
  }

  @override
  void dispose() {
    _disposed = true;
    _cancelPolicy();
    _cancelProfile();
    _nextGeneration();
    unawaited(_codes?.cancel());
    unawaited(_failures?.cancel());
    super.dispose();
  }
}
