import 'dart:async';
import 'dart:convert';

import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:uuid/uuid.dart';

import '../community/community.dart';

import '../storage/secure_storage.dart';
import '../storage/serial_queue.dart';
import 'builderlab_account.dart';
import 'known_community_address.dart';

/// Durable account-list edits. Community credentials and relay membership never
/// enter this queue. Each account keeps its own snapshot and latest URL intents.
class KnownCommunitySync extends ChangeNotifier {
  /// Reconciles committed, explicitly owned additions and queued removals.
  KnownCommunitySync({
    required this.account,
    required this.localCommunities,
    FlutterSecureStorage? storage,
    this.retryDelay = const Duration(seconds: 2),
  }) : _storage = storage ?? accountSecureStorage {
    account.subjectChanges.addListener(_sessionChanged);
    account.sessionChanges.addListener(_sessionChanged);
  }

  /// Account API whose verified session authorizes each destination edit.
  final BuilderlabAccountClient account;

  /// Durable device records, including their explicit account ownership.
  final Future<List<Community>> Function() localCommunities;
  final FlutterSecureStorage _storage;

  /// Initial transient retry delay; subsequent failures back off up to 32x.
  final Duration retryDelay;

  /// Latest failure for the current account; pending work remains durable.
  Object? get failure => _failure;
  Object? _failure;
  final _queue = SerialQueue();
  Timer? _retry;
  bool _disposed = false;
  bool _active = true;
  Future<void>? _work;
  bool _again = false;
  String? _blockedSubject;
  int _failures = 0;

  String _key(String subject) =>
      'buzz_known_communities_v1_${base64Url.encode(utf8.encode('${account.accountOrigin.origin}\n$subject'))}';

  Future<_SyncState> _read(String subject) async {
    final raw = await _storage.read(key: _key(subject));
    return raw == null ? _SyncState() : _SyncState.decode(raw);
  }

  Future<void> _write(String subject, _SyncState state) =>
      _storage.write(key: _key(subject), value: state.encode());

  bool _current(String subject) =>
      !_disposed && account.account?.subject == subject;

  void _sessionChanged() {
    _failure = null;
    _blockedSubject = null;
    _failures = 0;
    _retry?.cancel();
    notifyListeners();
    trigger();
  }

  /// Stop retries immediately when the app leaves the foreground.
  void setActive(bool active) {
    _active = active;
    _retry?.cancel();
    if (active) trigger();
  }

  /// Persist removal before deleting the local record. The drain sends it only
  /// once the local record is absent, so failed local deletion cannot forget it
  /// remotely. Signed-out edits never borrow another account's credentials.
  Future<void> remove(String relay) async {
    final subject = account.account?.subject;
    final url = knownCommunityAddress(relay);
    if (subject == null || url == null) return;
    await _queue.run(() async {
      final state = await _read(subject);
      if (!_current(subject)) throw AccountFailure.differentAccount;
      final local = await localCommunities();
      if (!_current(subject)) throw AccountFailure.differentAccount;
      for (final item in local) {
        if (knownCommunityAddress(item.relayUrl) == url &&
            item.accountSyncOwner == _owner(subject) &&
            item.accountSyncRevision != null) {
          state.observed[url] = item.accountSyncRevision!;
        }
      }
      state.pending[url] = true;
      await _write(subject, state);
    });
    if (!_disposed) notifyListeners();
  }

  String _owner(String subject) => '${account.accountOrigin.origin}\n$subject';

  /// Stamp only explicit additions. Saving this returned record atomically with
  /// credentials is the durable add intent; a later journal failure cannot turn
  /// a completed join into an authentication failure. Unclaimed legacy records
  /// and another account's retained records are never automatically uploaded.
  Community prepareAddition(Community community) {
    final subject = account.account?.subject;
    if (subject == null &&
        community.accountSyncOwner == null &&
        community.accountSyncRevision == null) {
      return community;
    }
    return community.copyWith(
      accountSyncOwner: subject == null ? null : _owner(subject),
      accountSyncRevision: subject == null ? null : const Uuid().v4(),
    );
  }

  /// An explicit retry clears a refused request without dropping its intent.
  void retry() {
    _blockedSubject = null;
    _failure = null;
    _failures = 0;
    notifyListeners();
    trigger();
  }

  /// Pending removals for visible status, including across app restarts.
  Future<Set<String>> pendingRemovals(String subject) => _queue.run(() async {
    final state = await _read(subject);
    return state.pending.entries
        .where((entry) => entry.value)
        .map((entry) => entry.key)
        .toSet();
  });

  /// Run after startup, a saved-community change, sign-in, or app resume.
  void trigger() => unawaited(synchronize());

  /// Coalesce concurrent refreshes and complete after their queued work drains.
  Future<void> synchronize() {
    if (_disposed || !_active) return Future.value();
    _retry?.cancel();
    final work = _work;
    if (work != null) {
      _again = true;
      return work;
    }
    return _work = _drain().whenComplete(() => _work = null);
  }

  Future<void> _drain() async {
    do {
      _again = false;
      await _run();
    } while (_again && !_disposed && _active);
  }

  Future<void> _run() async {
    String? attemptSubject;
    try {
      final session = account.account ?? await account.restore();
      if (session == null || !_current(session.subject) || !_active) return;
      final subject = session.subject;
      attemptSubject = subject;
      if (_blockedSubject == subject) return;
      var local = <String>{};
      await _queue.run(() async {
        final records = await localCommunities();
        local = records
            .map((c) => knownCommunityAddress(c.relayUrl))
            .whereType<String>()
            .toSet();
        final state = await _read(subject);
        if (!_current(subject)) return;
        final owned = records
            .where(
              (item) =>
                  item.accountSyncOwner == _owner(subject) &&
                  item.accountSyncRevision != null,
            )
            .map((item) => knownCommunityAddress(item.relayUrl))
            .whereType<String>()
            .toSet();
        // Older preview journals had no ownership evidence for inferred adds.
        state.pending.removeWhere(
          (url, removed) => !removed && !owned.contains(url),
        );
        for (final item in records) {
          final url = knownCommunityAddress(item.relayUrl);
          final revision = item.accountSyncRevision;
          if (url == null ||
              revision == null ||
              item.accountSyncOwner != _owner(subject)) {
            continue;
          }
          if (state.observed[url] != revision) {
            // A committed re-add wins over an older removal, even if the app
            // stopped before it could update this separate retry journal.
            state.pending[url] = false;
            state.observed[url] = revision;
          }
        }
        await _write(subject, state);
      });
      while (_current(subject) && _active) {
        final pending = await _queue.run(() => _read(subject));
        final entries = pending.pending.entries.where(
          (entry) => !entry.value || !local.contains(entry.key),
        );
        if (entries.isEmpty) break;
        final op = entries.first;
        if (!_current(subject)) return;
        await account.updateKnownCommunity(
          op.key,
          removed: op.value,
          expectedSubject: subject,
        );
        if (!_current(subject)) return;
        await _queue.run(() async {
          final latest = await _read(subject);
          // An edit made while the request was in flight wins over its ack.
          if (latest.pending[op.key] == op.value) {
            latest.pending.remove(op.key);
            await _write(subject, latest);
          }
        });
        if (!_disposed) notifyListeners();
      }
      _failures = 0;
      if (_failure != null) {
        _failure = null;
        notifyListeners();
      }
    } catch (error) {
      if (_disposed || (attemptSubject != null && !_current(attemptSubject))) {
        return;
      }
      _failure = error;
      notifyListeners();
      if (error == AccountFailure.denied ||
          error == AccountFailure.rejected ||
          error == AccountFailure.invalidResponse ||
          error == AccountFailure.accountSetupRequired) {
        // Preserve intent but do not hammer a refused API until a new sign-in.
        _blockedSubject = account.account?.subject;
      } else if (_active && error != AccountFailure.signedOut) {
        final multiplier = 1 << (_failures++).clamp(0, 5);
        _retry = Timer(retryDelay * multiplier, trigger);
      }
      debugPrint('Account community sync deferred: $error');
    }
  }

  @override
  void dispose() {
    _disposed = true;
    _retry?.cancel();
    account.subjectChanges.removeListener(_sessionChanged);
    account.sessionChanges.removeListener(_sessionChanged);
    super.dispose();
  }
}

class _SyncState {
  _SyncState({Map<String, String>? observed, Map<String, bool>? pending})
    : observed = observed ?? {},
      pending = pending ?? {};
  final Map<String, String> observed;
  final Map<String, bool> pending;

  factory _SyncState.decode(String raw) {
    final value = jsonDecode(raw) as Map<String, dynamic>;
    return _SyncState(
      observed: (value['observed'] as Map?)?.cast<String, String>() ?? {},
      pending: (value['pending'] as Map).cast<String, bool>(),
    );
  }

  String encode() => jsonEncode({'observed': observed, 'pending': pending});
}
