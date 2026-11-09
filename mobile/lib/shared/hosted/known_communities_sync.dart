import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../storage/secure_storage.dart';
import '../storage/serial_queue.dart';
import 'builderlab_account.dart';
import 'hosted_environment.dart';

/// A destination the way the account service spells it: `wss://host[:port]`
/// with a lowercase host, no default port and no trailing slash. Null for a
/// relay the service could never hold (insecure, malformed, or carrying
/// credentials, a path, a query or a fragment).
String? knownCommunityAddress(String relayUrl) {
  final uri = Uri.tryParse(relayUrl.trim());
  if (uri == null ||
      !const {'wss', 'https'}.contains(uri.scheme) ||
      uri.userInfo.isNotEmpty ||
      uri.hasQuery ||
      uri.hasFragment ||
      (uri.path.isNotEmpty && uri.path != '/')) {
    return null;
  }
  final host = uri.host.toLowerCase().replaceFirst(RegExp(r'\.+$'), '');
  // The same host shape `loadKnownHostedCommunities` accepts. Desktop's
  // address is `new URL(...).host` unfiltered, so a punycode (`xn--`) or
  // underscore host it lists reads as unsaveable here: such a community
  // joined on desktop never reaches or leaves this device.
  if (!RegExp(r'^[a-z0-9]+(?:[.-][a-z0-9]+)*$').hasMatch(host)) return null;
  final port = uri.hasPort && uri.port != 443 ? ':${uri.port}' : '';
  return 'wss://$host$port';
}

/// One intent awaiting upload: the latest word on its destination. A newer
/// intent replaces it outright, which is safe because the service's add and
/// remove are idempotent: whichever of them lands last is the state.
@immutable
class KnownCommunityIntent {
  const KnownCommunityIntent({required this.url, required this.removed});
  final String url;
  final bool removed;

  Map<String, dynamic> toJson() => {'url': url, 'removed': removed};

  @override
  bool operator ==(Object other) =>
      other is KnownCommunityIntent &&
      other.url == url &&
      other.removed == removed;

  @override
  int get hashCode => Object.hash(url, removed);

  @override
  String toString() => '${removed ? '-' : '+'}$url';
}

/// Durable known-community sync state, mirroring desktop's record. Pure:
/// every method returns a new state and never touches storage or the network.
@immutable
class KnownCommunitiesSyncState {
  const KnownCommunitiesSyncState({
    this.known = const [],
    this.outbox = const [],
    this.account,
  });

  static const empty = KnownCommunitiesSyncState();

  /// The destinations the service has been seen to hold for [account]: every
  /// one it has listed or accepted an add for, less the removals it has
  /// accepted from here. A list that omits one does not drop it, because a
  /// removal made elsewhere is deliberately ignored for now: the membership
  /// stays saved on this device and, being known, is never uploaded again.
  final List<String> known;
  final List<KnownCommunityIntent> outbox;

  /// The Builderlab account (its subject) whose list [known] describes. Absent
  /// until a first merge. The record outlives a sign-out, so a sign-in as
  /// another account must not read this set as that account's.
  final String? account;

  /// Reads a saved record. Records from before this field existed, or ones
  /// this reader cannot understand, read as empty; a malformed entry is
  /// dropped on its own.
  factory KnownCommunitiesSyncState.fromJson(Object? raw) {
    if (raw is! Map) return empty;
    final account = raw['account'];
    final known = raw['known'];
    final outbox = raw['outbox'];
    final intents = <String, KnownCommunityIntent>{};
    if (outbox is List) {
      for (final entry in outbox) {
        if (entry is Map &&
            entry['url'] is String &&
            (entry['url'] as String).startsWith('wss://') &&
            entry['removed'] is bool) {
          final url = entry['url'] as String;
          intents.remove(url);
          intents[url] = KnownCommunityIntent(
            url: url,
            removed: entry['removed'] as bool,
          );
        }
      }
    }
    return KnownCommunitiesSyncState(
      account: account is String && account.isNotEmpty ? account : null,
      known: known is List
          ? List.unmodifiable({
              for (final value in known)
                if (value is String && value.startsWith('wss://')) value,
            })
          : const [],
      outbox: List.unmodifiable(intents.values),
    );
  }

  Map<String, dynamic> toJson() => {
    if (account != null) 'account': account,
    'known': known,
    'outbox': [for (final op in outbox) op.toJson()],
  };

  KnownCommunityIntent? pendingFor(String url) =>
      outbox.where((op) => op.url == url).firstOrNull;

  /// Records a new intent for a destination, replacing any older one. Nothing
  /// is queued when the latest word already says so: the pending intent, or
  /// the service holding the destination. Removing a destination the service
  /// is not known to hold still queues, in case another device saved it.
  KnownCommunitiesSyncState enqueue(String url, {required bool removed}) {
    final pending = pendingFor(url);
    final bool? latest = pending != null
        ? pending.removed
        : known.contains(url)
        ? false
        : null;
    if (latest == removed) return this;
    return KnownCommunitiesSyncState(
      known: known,
      account: account,
      outbox: List.unmodifiable([
        ...outbox.where((op) => op.url != url),
        KnownCommunityIntent(url: url, removed: removed),
      ]),
    );
  }

  /// The service accepted [op]: it now holds the destination or no longer
  /// does. The intent leaves the outbox unless a different one replaced it.
  KnownCommunitiesSyncState acknowledge(KnownCommunityIntent op) {
    final held = known.contains(op.url);
    return KnownCommunitiesSyncState(
      account: account,
      known: op.removed
          ? (held
                ? List.unmodifiable(known.where((url) => url != op.url))
                : known)
          : (held ? known : List.unmodifiable([...known, op.url])),
      outbox: List.unmodifiable(outbox.where((entry) => entry != op)),
    );
  }

  /// Reconciles [account]'s complete server list with the device's
  /// [memberships] (already in service spelling). The list joins what is
  /// known. A destination the service has that the device lacks is returned
  /// in `add` unless its removal is queued; a saved membership the service has
  /// never seen is queued for upload. A saved membership the service knew but
  /// no longer lists is left alone: not forgotten and, still known, not
  /// uploaded again, so this device never undoes another's removal. What was
  /// known describes only the account it was merged for: another account's
  /// list starts from nothing known and uploads every membership it lacks.
  ({KnownCommunitiesSyncState state, List<String> add}) merge({
    required Iterable<String> memberships,
    required List<String> listed,
    required String account,
  }) {
    final before = this.account == account ? known : const <String>[];
    final merged = <String>{...before, ...listed};
    final local = memberships.toSet();
    final add = <String>[
      for (final url in {...listed})
        if (!local.contains(url) && pendingFor(url)?.removed != true) url,
    ];
    var next = KnownCommunitiesSyncState(
      known: List.unmodifiable(merged),
      outbox: outbox,
      account: account,
    );
    for (final url in local) {
      if (!merged.contains(url) && pendingFor(url) == null) {
        next = next.enqueue(url, removed: false);
      }
    }
    return (state: next, add: add);
  }
}

/// Durable intents and the known set, one record per device. Joins and leaves
/// record here whether or not an account is signed in; the sync owner drains
/// the record only while one is.
class KnownCommunitiesOutbox extends ChangeNotifier {
  KnownCommunitiesOutbox({FlutterSecureStorage? storage})
    : _storage = storage ?? communitySecureStorage;

  static const storageKey = 'buzz_known_communities_sync_v1';
  final FlutterSecureStorage _storage;
  final _queue = SerialQueue();

  Future<KnownCommunitiesSyncState> read() => _queue.run(_read);

  Future<KnownCommunitiesSyncState> _read() async {
    final raw = await _storage.read(key: storageKey);
    if (raw == null) return KnownCommunitiesSyncState.empty;
    try {
      return KnownCommunitiesSyncState.fromJson(jsonDecode(raw));
    } on FormatException {
      return KnownCommunitiesSyncState.empty;
    }
  }

  Future<void> _write(KnownCommunitiesSyncState state) =>
      _storage.write(key: storageKey, value: jsonEncode(state.toJson()));

  /// Records a join ([removed] false) or leave of [relayUrl]. A relay the
  /// service could never hold records nothing. Notifies listeners when a
  /// fresh intent was queued, so the owner sends it at once.
  Future<void> record(String relayUrl, {required bool removed}) async {
    final url = knownCommunityAddress(relayUrl);
    if (url == null) return;
    final fresh = await _queue.run(() async {
      final before = await _read();
      final after = before.enqueue(url, removed: removed);
      if (identical(after, before)) return false;
      await _write(after);
      return true;
    });
    if (fresh) notifyListeners();
  }

  /// Applies the server list for [account]; returns the destinations to add
  /// locally. The record is re-read inside the write so an intent queued
  /// while the list was in flight is kept.
  Future<List<String>> merge({
    required Iterable<String> memberships,
    required List<String> listed,
    required String account,
  }) => _queue.run(() async {
    final merged = (await _read()).merge(
      memberships: memberships,
      listed: listed,
      account: account,
    );
    await _write(merged.state);
    return merged.add;
  });

  Future<void> acknowledge(KnownCommunityIntent op) => _queue.run(() async {
    await _write((await _read()).acknowledge(op));
  });
}

/// Why the service refused a request. None is retried as sent.
enum KnownCommunitiesRefusal {
  /// This account may not sync.
  forbidden,

  /// The service has no such route: not yet deployed, or only partly.
  missing,

  /// The account's setup is incomplete.
  accountSetupRequired,

  /// A client error for the request as sent (a refused address, a full
  /// account): the same request cannot succeed.
  rejected,

  /// A success whose body this client cannot read: a contract the two sides
  /// no longer share.
  invalidResponse,
}

class KnownCommunitiesRefused implements Exception {
  const KnownCommunitiesRefused(this.kind, {this.status});
  final KnownCommunitiesRefusal kind;
  final int? status;

  /// Whether this refusal stops syncing until the next sign-in rather than
  /// parking one intent.
  bool get halts => const {
    KnownCommunitiesRefusal.forbidden,
    KnownCommunitiesRefusal.missing,
    KnownCommunitiesRefusal.accountSetupRequired,
  }.contains(kind);

  @override
  String toString() => status == null ? kind.name : 'HTTP $status';
}

/// The account's list and whether it names an identity this device lacks.
class KnownCommunitiesListing {
  const KnownCommunitiesListing(this.communities, {this.dropped = 0});

  /// In service spelling, deduplicated.
  final List<String> communities;

  /// Rows whose address this device could never save, left out.
  final int dropped;
}

/// The known-communities routes under the signed-in Builderlab session.
/// Transient failures ([AccountFailure.unavailable], storage) propagate for
/// retry; a service refusal is a [KnownCommunitiesRefused]; a session that
/// ended or changed propagates its [AccountFailure] for the owner to abandon.
class KnownCommunitiesClient {
  const KnownCommunitiesClient(this.account);
  final BuilderlabAccountClient account;

  static const _prefix = '/v1/buzz/known-communities';

  Future<KnownCommunitiesListing> list(String subject) async {
    final body = await _post('list', {}, subject);
    // Protobuf JSON omits an empty repeated field. A body without the field
    // is an empty list only when it is an object the service answered with.
    final rows = body['communities'] ?? const <dynamic>[];
    if (rows is! List) {
      throw const KnownCommunitiesRefused(
        KnownCommunitiesRefusal.invalidResponse,
      );
    }
    final communities = <String>{};
    var dropped = 0;
    for (final row in rows) {
      if (row is! Map || row['community_url'] is! String) {
        throw const KnownCommunitiesRefused(
          KnownCommunitiesRefusal.invalidResponse,
        );
      }
      final raw = row['community_url'] as String;
      final address = raw.startsWith('wss://')
          ? knownCommunityAddress(raw)
          : null;
      if (address == null) {
        dropped++;
      } else {
        communities.add(address);
      }
    }
    return KnownCommunitiesListing(
      List.unmodifiable(communities),
      dropped: dropped,
    );
  }

  /// Both edits are idempotent, so a lost answer is retried as sent.
  Future<void> add(String url, String subject) =>
      _post('add', {'community_url': url}, subject);

  Future<void> remove(String url, String subject) =>
      _post('remove', {'community_url': url}, subject);

  Future<Map<String, dynamic>> _post(
    String route,
    Map<String, dynamic> body,
    String subject,
  ) async {
    final Map<String, dynamic> result;
    try {
      result = await account.post(
        '$_prefix/$route',
        body,
        expectedSubject: subject,
      );
    } on AccountFailure catch (failure) {
      switch (failure) {
        case AccountFailure.denied:
          throw const KnownCommunitiesRefused(
            KnownCommunitiesRefusal.forbidden,
            status: 403,
          );
        case AccountFailure.accountSetupRequired:
          throw const KnownCommunitiesRefused(
            KnownCommunitiesRefusal.accountSetupRequired,
          );
        case AccountFailure.rejected:
          final status = account.lastRejectedRequest?.status;
          throw KnownCommunitiesRefused(
            status == 404
                ? KnownCommunitiesRefusal.missing
                : KnownCommunitiesRefusal.rejected,
            status: status,
          );
        case AccountFailure.invalidResponse:
          throw const KnownCommunitiesRefused(
            KnownCommunitiesRefusal.invalidResponse,
          );
        case AccountFailure.signedOut:
        case AccountFailure.differentAccount:
        case AccountFailure.unavailable:
        case AccountFailure.storage:
          rethrow;
      }
    }
    // The service names its refusals in the body of an otherwise successful
    // answer. A named `forbidden` means this account may not sync, as a 403
    // does; any other name refuses the request as sent.
    final error = result['error'];
    if (error != null) {
      throw KnownCommunitiesRefused(
        error == 'forbidden'
            ? KnownCommunitiesRefusal.forbidden
            : KnownCommunitiesRefusal.rejected,
      );
    }
    return result;
  }
}

/// The signed-in identity cannot admit any destination yet: it is not set
/// up, not backed up or not bound. Every join waits for the next trigger
/// rather than retrying on a timer.
class KnownCommunitiesNotReady implements Exception {
  const KnownCommunitiesNotReady();
}

/// This destination alone cannot be joined by the signed-in identity now: it
/// is saved here under another key, or its relay refused the identity. The
/// destinations after it proceed; this one counts toward
/// [KnownCommunitiesSync.maxAdmissionAttempts] and waits for the next trigger.
class KnownCommunityAdmissionDenied implements Exception {
  const KnownCommunityAdmissionDenied();
}

/// Joins [url] on this device for [subject]; see [KnownCommunitiesNotReady]
/// and [KnownCommunityAdmissionDenied].
typedef KnownCommunityAdmission =
    Future<void> Function(
      String url,
      String subject,
      bool Function() isCurrent,
    );

typedef KnownCommunitiesTimer =
    Timer Function(Duration delay, void Function() wake);

/// One sign-in's worth of sync progress; replaced by the next sign-in.
class _Span {
  _Span(this.subject);
  final String subject;
  bool listed = false;
  bool halted = false;
  int failures = 0;

  /// Refused intents, by [KnownCommunitiesSync._key], and destinations this
  /// sign-in will not join: ones whose local admission kept failing and ones
  /// the user left unchosen in the picker. All wait for the next sign-in.
  final parked = <String>{};
  final pendingAdds = <String>[];
  final admissionFailures = <String, int>{};
}

/// Keeps the account's known-community list in step with this device while a
/// Builderlab account is signed in, as desktop's known-communities sync does.
/// Each sign-in reads the complete list, merges it (add-only: a membership
/// the list omits is kept and not re-uploaded, and what an earlier sign-in as
/// another account knew is not read as this one's list), joins each listed
/// destination this device lacks through hosted admission, then sends each
/// destination's latest intent one at a time. A newly queued intent, the
/// app resuming, or the hosted flow settling while joins wait on it runs it
/// again at once; a failure retries at 1·2·4… s, capped at a minute,
/// re-sending whatever intent then stands, which the service's idempotent
/// add and remove make safe. Refusals
/// are not retried: an account that cannot sync, a list this client cannot
/// read, or a service without the routes (404) stops everything until the
/// next sign-in, while a refused address or another client error parks that
/// one intent until it is replaced or the next sign-in. A destination whose
/// local admission fails [maxAdmissionAttempts] times, or that the user saw
/// in the picker and left unchosen ([decline]), is parked the same way.
/// Signing out or disposal abandons in-flight work and leaves the outbox.
class KnownCommunitiesSync {
  KnownCommunitiesSync({
    required this.account,
    required this.outbox,
    required this.client,
    required this.memberships,
    required this.admit,
    bool Function()? canAdmit,
    bool Function(String url)? admittable,
    KnownCommunitiesTimer? timer,
    this.maxBackoff = const Duration(minutes: 1),
    this.maxAdmissionAttempts = 3,
  }) : _canAdmit = canAdmit ?? (() => true),
       _admittable =
           admittable ??
           ((url) => HostedEnvironment.supportsRelay(Uri.parse(url))),
       _timer = timer ?? Timer.new;

  final BuilderlabAccountClient account;
  final KnownCommunitiesOutbox outbox;
  final KnownCommunitiesClient client;

  /// The relay URLs of every community saved on this device.
  final Future<List<String>> Function() memberships;
  final KnownCommunityAdmission admit;
  final bool Function() _canAdmit;
  final bool Function(String url) _admittable;
  final KnownCommunitiesTimer _timer;
  final Duration maxBackoff;
  final int maxAdmissionAttempts;

  _Span? _span;
  Timer? _retry;
  bool _running = false;
  bool _rerun = false;
  bool _started = false;
  bool _disposed = false;
  Future<void>? _restoring;

  /// Whether a sign-in is being synced.
  bool get active => _span != null;

  /// Whether this sign-in's sync stopped on a refusal.
  @visibleForTesting
  bool get halted => _span?.halted ?? false;

  @visibleForTesting
  Set<String> get parked => Set.unmodifiable(_span?.parked ?? const {});

  /// Whether listed destinations are still waiting to be joined, because
  /// the identity was busy or not ready. The owner kicks when that changes.
  bool get hasDeferredJoins => _span?.pendingAdds.isNotEmpty ?? false;

  /// The user saw these listed destinations in the community picker and did
  /// not choose them: this sign-in does not join them in the background,
  /// whether the list named them already or names them later. The next
  /// sign-in lists them again. Nothing is recorded while signed out.
  void decline(Iterable<String> relayUrls) {
    final span = _span;
    if (span == null) return;
    for (final url in relayUrls.map(knownCommunityAddress).nonNulls) {
      span.parked.add(url);
      span.pendingAdds.remove(url);
    }
  }

  static String _key(KnownCommunityIntent op) => op.toString();

  Duration _backoff(int failures) => Duration(
    milliseconds: min(
      maxBackoff.inMilliseconds,
      1000 * (1 << min(failures, 20)),
    ),
  );

  /// Begins following the account. Idempotent.
  void start() {
    if (_disposed || _started) return;
    _started = true;
    account.subjectChanges.addListener(_onSubject);
    outbox.addListener(_onOutbox);
    _onSubject();
    if (_span == null) _restoreSession();
  }

  /// Runs now, or again once the request in flight settles, resetting the
  /// backoff. While signed out, checks for a saved session instead.
  void kick() {
    if (_disposed) return;
    final span = _span;
    if (span == null) {
      _restoreSession();
      return;
    }
    if (span.halted) return;
    span.failures = 0;
    _cancelRetry();
    if (_running) {
      _rerun = true;
    } else {
      unawaited(_run(span));
    }
  }

  /// A saved session adopts through [BuilderlabAccountClient.restore], which
  /// notifies [_onSubject]. Failures leave the account signed out here until
  /// the next trigger; concurrent triggers share one restore.
  void _restoreSession() {
    if (account.account != null || _restoring != null) return;
    _restoring = account
        .restore()
        .then<void>((_) {})
        .catchError((Object error) {
          debugPrint('Known-communities sync could not restore: $error');
        })
        .whenComplete(() => _restoring = null);
  }

  void _onSubject() {
    final subject = account.subjectChanges.value;
    final span = _span;
    if (subject == null) {
      if (span != null) {
        _span = null;
        _cancelRetry();
      }
      return;
    }
    if (span?.subject == subject) return;
    _cancelRetry();
    _span = _Span(subject);
    kick();
  }

  /// A new intent drains at once while signed in; signed out, it waits for
  /// the next sign-in rather than probing for a saved session.
  void _onOutbox() {
    if (_span != null) kick();
  }

  void _cancelRetry() {
    _retry?.cancel();
    _retry = null;
  }

  bool _current(_Span span) => !_disposed && identical(_span, span);

  Future<void> _run(_Span span) async {
    if (!_current(span) || span.halted || _running) return;
    _cancelRetry();
    _running = true;
    _rerun = false;
    try {
      if (!span.listed) await _list(span);
      if (!_current(span) || span.halted) return;
      await _admitPending(span);
      if (!_current(span) || span.halted) return;
      await _drain(span);
      if (!_current(span) || span.halted) return;
      span.failures = 0;
      // A parked intent that was replaced or withdrawn no longer counts, so
      // the same intent queued again later is sent.
      final pending = (await outbox.read()).outbox.map(_key).toSet();
      if (!_current(span)) return;
      span.parked.removeWhere(
        (key) =>
            (key.startsWith('+') || key.startsWith('-')) &&
            !pending.contains(key),
      );
    } on KnownCommunitiesRefused catch (refusal) {
      if (!_current(span)) return;
      _halt(span, 'list answered $refusal');
    } catch (error) {
      if (!_current(span)) return;
      if (error is AccountFailure &&
          (error == AccountFailure.signedOut ||
              error == AccountFailure.differentAccount)) {
        // The session ended or changed; the account notifies in turn.
        return;
      }
      debugPrint('Known-communities sync failed: $error');
      _retry = _timer(_backoff(span.failures++), () {
        _retry = null;
        unawaited(_run(span));
      });
    } finally {
      _running = false;
      if (_rerun) {
        final current = _span;
        if (current != null) unawaited(_run(current));
      }
    }
  }

  void _halt(_Span span, String what) {
    debugPrint(
      'Builderlab $what; known-communities sync stops until the next sign-in.',
    );
    span.halted = true;
    _cancelRetry();
  }

  Future<void> _list(_Span span) async {
    final listing = await client.list(span.subject);
    if (!_current(span)) return;
    if (listing.dropped > 0) {
      debugPrint(
        'Builderlab listed ${listing.dropped} community address(es) this device can’t save; they were left out.',
      );
    }
    final saved = await memberships();
    if (!_current(span)) return;
    final local = saved.map(knownCommunityAddress).nonNulls.toSet();
    final add = await outbox.merge(
      memberships: local,
      listed: listing.communities,
      account: span.subject,
    );
    if (!_current(span)) return;
    span.listed = true;
    for (final url in add) {
      if (span.parked.contains(url)) continue;
      if (_admittable(url)) {
        if (!span.pendingAdds.contains(url)) span.pendingAdds.add(url);
      } else {
        debugPrint(
          'Builderlab lists $url, which this build cannot join on its own.',
        );
      }
    }
  }

  /// Joins listed destinations one at a time while the hosted flow is not
  /// using the identity. A destination joined meanwhile (by the picker, say)
  /// is done. An identity that is not ready waits for a trigger; one this
  /// destination alone cannot use skips to the next destination and also
  /// waits for a trigger. A destination that keeps failing either way is
  /// parked for this sign-in.
  Future<void> _admitPending(_Span span) async {
    if (span.pendingAdds.isEmpty) return;
    final saved = (await memberships())
        .map(knownCommunityAddress)
        .nonNulls
        .toSet();
    if (!_current(span)) return;
    span.pendingAdds.removeWhere(saved.contains);
    for (final url in List.of(span.pendingAdds)) {
      if (!_current(span)) return;
      if (!_canAdmit()) return;
      try {
        await admit(url, span.subject, () => _current(span));
        if (!_current(span)) return;
        span.pendingAdds.remove(url);
      } on KnownCommunitiesNotReady {
        return;
      } on KnownCommunityAdmissionDenied catch (denial) {
        if (!_current(span)) return;
        _admissionFailed(span, url, denial);
      } catch (error) {
        if (!_current(span)) return;
        if (!_admissionFailed(span, url, error)) rethrow;
      }
    }
  }

  /// Counts a failed attempt to join [url]; parks it for this sign-in once
  /// it has failed [maxAdmissionAttempts] times. Returns whether it parked.
  bool _admissionFailed(_Span span, String url, Object error) {
    final attempts = (span.admissionFailures[url] ?? 0) + 1;
    span.admissionFailures[url] = attempts;
    if (attempts < maxAdmissionAttempts) return false;
    debugPrint(
      'Joining $url failed $attempts times ($error); known-communities sync skips it until the next sign-in.',
    );
    span.pendingAdds.remove(url);
    span.parked.add(url);
    return true;
  }

  Future<void> _drain(_Span span) async {
    for (;;) {
      if (!_current(span)) return;
      final state = await outbox.read();
      if (!_current(span)) return;
      final op = state.outbox
          .where((entry) => !span.parked.contains(_key(entry)))
          .firstOrNull;
      if (op == null) return;
      try {
        if (op.removed) {
          await client.remove(op.url, span.subject);
        } else {
          await client.add(op.url, span.subject);
        }
      } on KnownCommunitiesRefused catch (refusal) {
        if (!_current(span)) return;
        final what = '${op.removed ? 'remove' : 'add'} ${op.url} answered';
        if (refusal.halts) {
          _halt(span, '$what $refusal');
          return;
        }
        debugPrint(
          'Builderlab $what $refusal; known-communities sync skips it until it is replaced or the next sign-in.',
        );
        span.parked.add(_key(op));
        continue;
      }
      if (!_current(span)) return;
      await outbox.acknowledge(op);
    }
  }

  void dispose() {
    if (_disposed) return;
    _disposed = true;
    _span = null;
    _cancelRetry();
    if (_started) {
      account.subjectChanges.removeListener(_onSubject);
      outbox.removeListener(_onOutbox);
    }
  }
}

/// The device's durable intents, shared by every join and leave seam.
final knownCommunitiesOutboxProvider = Provider<KnownCommunitiesOutbox>((ref) {
  final outbox = KnownCommunitiesOutbox();
  ref.onDispose(outbox.dispose);
  return outbox;
});
