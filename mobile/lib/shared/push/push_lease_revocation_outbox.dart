import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../storage/secure_storage.dart';

import '../community/community.dart';
import '../community/community_provider.dart';
import '../relay/signed_event_relay.dart';
import '../relay/relay_provider.dart' show Env;
import 'dev_push_lease.dart';
import 'push_bridge.dart';

const _maxSafeJsonInteger = 9007199254740991;
const _baseRetryDelay = Duration(seconds: 30);
const _maximumRetryDelay = Duration(hours: 6);
const _lowercaseHex64Pattern = r'^[0-9a-f]{64}$';
const _installationIdPattern = r'^[0-9a-f]{32}$';

typedef BuzzPushLeaseRevocationPublisher =
    Future<void> Function(BuzzPushLeaseRevocationRecord record);
typedef BuzzPushLeaseRevocationClock = DateTime Function();
typedef BuzzPushLeaseRevocationJitter = double Function();
typedef BuzzPushLeaseRevocationErrorReporter =
    void Function(Object error, StackTrace stackTrace);
typedef BuzzPushLeaseRevocationWakeScheduler =
    void Function() Function(Duration delay, void Function() wake);

/// Durable state for retracting one precise NIP-PL lease address.
///
/// The signing key is intentionally retained in secure storage only until the
/// relay accepts the tombstone or the old endpoint grant expires.
@immutable
class BuzzPushLeaseRevocationRecord {
  final String relayUrl;
  final String relayOrigin;
  final String memberPubkey;
  final String nsec;
  final String installationId;

  /// Generation reserved for the next relay attempt.
  final int generation;
  final int expiresAt;
  final int attemptCount;
  final int nextAttemptAt;

  /// Exact gateway target retained until direct revocation succeeds.
  final Map<String, dynamic>? gatewayRevocation;
  final bool gatewayOnly;

  BuzzPushLeaseRevocationRecord({
    required this.relayUrl,
    required this.relayOrigin,
    required this.memberPubkey,
    required this.nsec,
    required this.installationId,
    required this.generation,
    required this.expiresAt,
    required this.attemptCount,
    required this.nextAttemptAt,
    this.gatewayRevocation,
    this.gatewayOnly = false,
  }) {
    if (canonicalBuzzPushRelayOrigin(relayUrl) != relayOrigin) {
      throw const FormatException(
        'Push revocation relay URL and origin do not match.',
      );
    }
    if (!RegExp(_lowercaseHex64Pattern).hasMatch(memberPubkey)) {
      throw const FormatException(
        'Push revocation member pubkey must be exact lowercase hex.',
      );
    }
    if (!gatewayOnly) {
      final decoded = nostr.Nip19.decode(payload: nsec);
      if (decoded.prefix != nostr.Nip19Prefix.nsec ||
          decoded.data.length != 64 ||
          derivePublicKeyHex(decoded.data) != memberPubkey) {
        throw const FormatException(
          'Push revocation signing key does not match its member pubkey.',
        );
      }
    }
    if (!RegExp(_installationIdPattern).hasMatch(installationId)) {
      throw const FormatException(
        'Push revocation installation id must be exact lowercase hex.',
      );
    }
    if (gatewayOnly && gatewayRevocation == null) {
      throw const FormatException('Gateway cleanup requires a target.');
    }
    if (gatewayRevocation case final target?) {
      const keys = {
        'gatewayUrl',
        'installationHandle',
        'keyId',
        'relayPubkey',
        'generation',
      };
      final pendingKeys = {
        'gatewayUrl',
        'relayOrigin',
        'relayPubkey',
        'generation',
      };
      final fields = target.containsKey('relayOrigin') ? pendingKeys : keys;
      if (!setEquals(target.keys.toSet(), fields) ||
          fields
              .where((key) => key != 'generation')
              .any((key) => target[key] is! String) ||
          target['generation'] is! int ||
          (target['generation'] as int) <= 0) {
        throw const FormatException('Invalid gateway revocation target.');
      }
    }
    if (generation <= 0 || generation > _maxSafeJsonInteger) {
      throw const FormatException('Push revocation generation is invalid.');
    }
    if (expiresAt <= 0 || attemptCount < 0 || nextAttemptAt < 0) {
      throw const FormatException('Push revocation retry state is invalid.');
    }
  }

  String get leaseAddress => gatewayOnly
      ? 'gateway|${gatewayRevocation?['gatewayUrl']}|$relayOrigin|$installationId|${gatewayRevocation?['relayPubkey']}|${gatewayRevocation?['installationHandle'] ?? 'pending'}'
      : '$memberPubkey|$relayOrigin|$installationId';

  BuzzPushLeaseRevocationRecord copyWith({
    int? generation,
    int? expiresAt,
    int? attemptCount,
    int? nextAttemptAt,
    bool? gatewayOnly,
    bool omitGatewayRevocation = false,
  }) => BuzzPushLeaseRevocationRecord(
    relayUrl: relayUrl,
    relayOrigin: relayOrigin,
    memberPubkey: memberPubkey,
    nsec: nsec,
    installationId: installationId,
    generation: generation ?? this.generation,
    expiresAt: expiresAt ?? this.expiresAt,
    attemptCount: attemptCount ?? this.attemptCount,
    nextAttemptAt: nextAttemptAt ?? this.nextAttemptAt,
    gatewayRevocation: omitGatewayRevocation ? null : gatewayRevocation,
    gatewayOnly: gatewayOnly ?? this.gatewayOnly,
  );

  Map<String, dynamic> toJson() => {
    'version': 1,
    'relayUrl': relayUrl,
    'relayOrigin': relayOrigin,
    'memberPubkey': memberPubkey,
    'nsec': nsec,
    'installationId': installationId,
    'generation': generation,
    'expiresAt': expiresAt,
    'attemptCount': attemptCount,
    'nextAttemptAt': nextAttemptAt,
    if (gatewayRevocation != null) 'gatewayRevocation': gatewayRevocation,
    if (gatewayOnly) 'gatewayOnly': true,
  };

  factory BuzzPushLeaseRevocationRecord.fromJson(Map<String, dynamic> json) {
    const keys = {
      'version',
      'relayUrl',
      'relayOrigin',
      'memberPubkey',
      'nsec',
      'installationId',
      'generation',
      'expiresAt',
      'attemptCount',
      'nextAttemptAt',
    };
    if (json.keys.toSet().difference({
          ...keys,
          'gatewayRevocation',
          'gatewayOnly',
        }).isNotEmpty ||
        keys.difference(json.keys.toSet()).isNotEmpty ||
        json['version'] != 1 ||
        json['relayUrl'] is! String ||
        json['relayOrigin'] is! String ||
        json['memberPubkey'] is! String ||
        json['nsec'] is! String ||
        json['installationId'] is! String ||
        json['generation'] is! int ||
        json['expiresAt'] is! int ||
        json['attemptCount'] is! int ||
        json['nextAttemptAt'] is! int) {
      throw const FormatException('Invalid push revocation record.');
    }
    return BuzzPushLeaseRevocationRecord(
      relayUrl: json['relayUrl'] as String,
      relayOrigin: json['relayOrigin'] as String,
      memberPubkey: json['memberPubkey'] as String,
      nsec: json['nsec'] as String,
      installationId: json['installationId'] as String,
      generation: json['generation'] as int,
      expiresAt: json['expiresAt'] as int,
      attemptCount: json['attemptCount'] as int,
      nextAttemptAt: json['nextAttemptAt'] as int,
      gatewayOnly: json['gatewayOnly'] == true,
      gatewayRevocation: json['gatewayRevocation'] == null
          ? null
          : Map<String, dynamic>.from(json['gatewayRevocation'] as Map),
    );
  }
}

class BuzzPushLeaseRevocationStorage {
  static const _key = 'buzz_push_lease_revocations_v1';

  final FlutterSecureStorage _secure;

  BuzzPushLeaseRevocationStorage({FlutterSecureStorage? secure})
    : _secure = secure ?? communitySecureStorage;

  Future<List<BuzzPushLeaseRevocationRecord>> loadAll() async {
    final raw = await _secure.read(key: _key);
    if (raw == null) return <BuzzPushLeaseRevocationRecord>[];
    final decoded = jsonDecode(raw);
    if (decoded is! List<dynamic>) {
      throw const FormatException('Push revocation outbox must be a list.');
    }
    final records = [
      for (final value in decoded)
        BuzzPushLeaseRevocationRecord.fromJson(
          Map<String, dynamic>.from(value as Map),
        ),
    ];
    final addresses = records.map((record) => record.leaseAddress).toSet();
    if (addresses.length != records.length) {
      throw const FormatException(
        'Push revocation outbox contains duplicate lease addresses.',
      );
    }
    return records;
  }

  Future<void> replaceAll(
    Iterable<BuzzPushLeaseRevocationRecord> records,
  ) async {
    final values = records.toList();
    final addresses = values.map((record) => record.leaseAddress).toSet();
    if (addresses.length != values.length) {
      throw const FormatException(
        'Push revocation outbox contains duplicate lease addresses.',
      );
    }
    if (values.isEmpty) {
      await _secure.delete(key: _key);
      return;
    }
    await _secure.write(
      key: _key,
      value: jsonEncode([for (final record in values) record.toJson()]),
    );
  }
}

/// A durable, single-flight retry coordinator for removed-community leases.
///
/// Every attempt reserves its successor generation and retry time before
/// network I/O. A process death during an ambiguous publication therefore
/// cannot cause an immediate replay or reuse a generation the relay may have
/// accepted.
class BuzzPushLeaseRevocationOutbox {
  BuzzPushLeaseRevocationOutbox({
    required this.storage,
    required this.publisher,
    this.now = DateTime.now,
    BuzzPushLeaseRevocationJitter? jitter,
    this.reportError = reportPushLeaseCleanupError,
    BuzzPushLeaseRevocationWakeScheduler? scheduleWake,
  }) : jitter = jitter ?? Random.secure().nextDouble,
       scheduleWake = scheduleWake ?? _scheduleTimer;

  final BuzzPushLeaseRevocationStorage storage;
  final BuzzPushLeaseRevocationPublisher publisher;
  final BuzzPushLeaseRevocationClock now;
  final BuzzPushLeaseRevocationJitter jitter;
  final BuzzPushLeaseRevocationErrorReporter reportError;
  final BuzzPushLeaseRevocationWakeScheduler scheduleWake;

  Future<void> _storageTail = Future.value();
  Future<void>? _drain;
  int _wakeGeneration = 0;
  void Function()? _cancelWake;
  bool _started = false;
  bool _disposed = false;

  Future<T> _serialize<T>(Future<T> Function() operation) {
    final result = _storageTail.then((_) => operation());
    _storageTail = result.then<void>(
      (_) {},
      onError: (Object _, StackTrace _) {},
    );
    return result;
  }

  Future<void> enqueue(BuzzPushLeaseRevocationRecord record) async {
    await _serialize(() async {
      final records = await storage.loadAll();
      // Save both authorities together before either attempt can invalidate
      // native inventory. Each record then retains only its unfinished work.
      final independent = [
        if (!record.gatewayOnly && record.gatewayRevocation != null)
          record.copyWith(gatewayOnly: true),
        record.gatewayOnly
            ? record
            : record.copyWith(omitGatewayRevocation: true),
      ];
      for (final cleanup in independent) {
        final index = records.indexWhere(
          (candidate) => candidate.leaseAddress == cleanup.leaseAddress,
        );
        if (index >= 0) {
          if (cleanup.generation <= records[index].generation) continue;
          records[index] = cleanup;
        } else {
          records.add(cleanup);
        }
      }
      await storage.replaceAll(records);
    });
    if (_started) _scheduleNextWake();
  }

  Future<bool> enqueueCommunity(
    Community community, {
    Future<List<BuzzPushEndpointGrant>> Function()? readGrants,
    List<Community> remainingCommunities = const [],
    bool gatewayOnly = false,
  }) async {
    if (gatewayOnly &&
        (!Env.pushGatewayConfigured ||
            defaultTargetPlatform != TargetPlatform.iOS)) {
      return false;
    }
    final state = community.pushSubscriptionState;
    final highestGeneration =
        state.generationCursor ?? state.acceptedGeneration;
    if (highestGeneration == null ||
        (!community.pushNotificationsEnabled &&
            state.pendingTombstoneGeneration == null)) {
      if (!Env.pushGatewayConfigured ||
          defaultTargetPlatform != TargetPlatform.iOS) {
        return false;
      }
      // Journal cancellation even before native enrollment has saved its first
      // record. The shared native lane cannot acknowledge while it is in flight.
      final relayOrigin = canonicalBuzzPushRelayOrigin(community.relayUrl);
      final seconds = now().millisecondsSinceEpoch ~/ 1000;
      await enqueue(
        BuzzPushLeaseRevocationRecord(
          relayUrl: canonicalBuzzPushRelayHttpUrl(community.relayUrl),
          relayOrigin: relayOrigin,
          memberPubkey: '0' * 64,
          nsec: '',
          installationId: community.pushLeaseInstallationId ?? '0' * 32,
          generation: 1,
          expiresAt: seconds + 300,
          attemptCount: 0,
          nextAttemptAt: seconds,
          gatewayOnly: true,
          gatewayRevocation: {
            'gatewayUrl': Env.pushGatewayUrl,
            'relayOrigin': relayOrigin,
            'relayPubkey': '',
            'generation': 1,
          },
        ),
      );
      return true;
    }
    final nsec = community.nsec;
    if (!gatewayOnly && (nsec == null || nsec.isEmpty)) {
      throw StateError(
        'Push lease revocation requires a community signing key.',
      );
    }
    final memberPubkey = gatewayOnly
        ? '0' * 64
        : community.pubkey ??
              derivePublicKeyHex(nostr.Nip19.decode(payload: nsec!).data);
    final relayUrl = canonicalBuzzPushRelayHttpUrl(community.relayUrl);
    final relayOrigin = canonicalBuzzPushRelayOrigin(relayUrl);
    final grants = await (readGrants ?? readBuzzPushEndpointGrants)();
    final matching = grants
        .where((grant) => grant.relayOrigin == relayOrigin)
        .toList();
    if (gatewayOnly && matching.isEmpty) return false;
    if (matching.length != 1) {
      throw StateError(
        'Expected exactly one endpoint grant for push lease revocation.',
      );
    }
    final currentSeconds = now().millisecondsSinceEpoch ~/ 1000;
    final grant = matching.single;
    final shared = remainingCommunities.any(
      (other) =>
          other.id != community.id &&
          other.pushNotificationsEnabled &&
          grants.any(
            (candidate) =>
                candidate.relayOrigin ==
                    canonicalBuzzPushRelayOrigin(other.relayUrl) &&
                candidate.relayPubkey == grant.relayPubkey &&
                (grant.pending
                    ? candidate.relayOrigin == grant.relayOrigin
                    : candidate.gatewayInstallationHandle ==
                          grant.gatewayInstallationHandle),
          ),
    );
    if (grant.expiresAt <= currentSeconds) return false;
    if (gatewayOnly && (shared || grant.gatewayRevocationTarget == null)) {
      return false;
    }
    if (highestGeneration + 2 > _maxSafeJsonInteger) {
      throw StateError('Push lease generation is exhausted.');
    }
    await enqueue(
      BuzzPushLeaseRevocationRecord(
        relayUrl: relayUrl,
        relayOrigin: relayOrigin,
        memberPubkey: memberPubkey,
        nsec: gatewayOnly ? '' : nsec!,
        installationId:
            community.pushLeaseInstallationId ?? grant.installationId,
        generation: highestGeneration + 2,
        expiresAt: grant.expiresAt,
        attemptCount: 0,
        nextAttemptAt: currentSeconds,
        gatewayRevocation: shared ? null : grant.gatewayRevocationTarget,
        gatewayOnly: gatewayOnly,
      ),
    );
    return true;
  }

  /// Reconciles disabled or removed origins against durable native authority,
  /// including first enrollment canceled before a relay lease was published.
  Future<void> reconcileGatewayCleanup(
    List<Community> communities, {
    Future<List<BuzzPushEndpointGrant>> Function()? readGrants,
  }) async {
    final grants = await (readGrants ?? readBuzzPushEndpointGrants)();
    for (final grant in grants) {
      final target = grant.gatewayRevocationTarget;
      if (target == null || (!grant.pending && grant.endpointGrant.isEmpty)) {
        continue;
      }
      final shared = communities.any(
        (community) =>
            community.pushNotificationsEnabled &&
            grants.any(
              (candidate) =>
                  candidate.relayOrigin ==
                      canonicalBuzzPushRelayOrigin(community.relayUrl) &&
                  candidate.relayPubkey == grant.relayPubkey &&
                  (grant.pending
                      ? candidate.relayOrigin == grant.relayOrigin
                      : candidate.gatewayInstallationHandle ==
                            grant.gatewayInstallationHandle),
            ),
      );
      if (shared) continue;
      await enqueue(
        BuzzPushLeaseRevocationRecord(
          relayUrl: canonicalBuzzPushRelayHttpUrl(grant.relayOrigin),
          relayOrigin: grant.relayOrigin,
          memberPubkey: '0' * 64,
          nsec: '',
          installationId: grant.installationId,
          generation: 1,
          expiresAt: grant.expiresAt,
          attemptCount: 0,
          nextAttemptAt: now().millisecondsSinceEpoch ~/ 1000,
          gatewayRevocation: target,
          gatewayOnly: true,
        ),
      );
    }
  }

  Future<void> start() async {
    if (_disposed) throw StateError('Push revocation outbox is disposed.');
    if (_started) return trigger();
    _started = true;
    await trigger();
  }

  Future<void> trigger() {
    if (_disposed) return Future.value();
    final active = _drain;
    if (active != null) return active;
    final drain = _drainDue();
    _drain = drain;
    void finish() {
      if (identical(_drain, drain)) _drain = null;
      if (!_disposed) _scheduleNextWake();
    }

    drain.then<void>((_) => finish(), onError: (_, _) => finish());
    return drain;
  }

  Future<void> _drainDue() async {
    while (!_disposed) {
      final record = await _reserveNextDueAttempt();
      if (record == null) return;
      try {
        await publisher(record);
        await _removeAccepted(record);
        pushLeaseCleanupError.value = null;
      } catch (error, stackTrace) {
        reportError(error, stackTrace);
      }
    }
  }

  Future<BuzzPushLeaseRevocationRecord?> _reserveNextDueAttempt() => _serialize(
    () async {
      final currentSeconds = now().millisecondsSinceEpoch ~/ 1000;
      final records = await storage.loadAll();
      final active = records
          .where(
            (record) =>
                record.expiresAt > currentSeconds ||
                record.gatewayRevocation != null,
          )
          .toList();
      final due =
          active
              .where((record) => record.nextAttemptAt <= currentSeconds)
              .toList()
            ..sort((left, right) {
              final schedule = left.nextAttemptAt.compareTo(
                right.nextAttemptAt,
              );
              return schedule != 0
                  ? schedule
                  : left.leaseAddress.compareTo(right.leaseAddress);
            });
      if (due.isEmpty) {
        if (active.length != records.length) {
          await storage.replaceAll(active);
        }
        return null;
      }
      final record = due.first;
      if (record.generation >= _maxSafeJsonInteger) {
        throw StateError('Push lease generation is exhausted.');
      }
      final reserved = record.copyWith(
        generation: record.generation + 1,
        attemptCount: record.attemptCount + 1,
        nextAttemptAt: currentSeconds + _retryDelaySeconds(record.attemptCount),
      );
      final index = active.indexWhere(
        (candidate) => candidate.leaseAddress == record.leaseAddress,
      );
      if (index < 0) {
        throw StateError('Reserved push revocation record disappeared.');
      }
      active[index] = reserved;
      await storage.replaceAll(active);
      return record;
    },
  );

  int _retryDelaySeconds(int priorAttempts) {
    final exponent = min(priorAttempts, 20);
    final factor = 1 << exponent;
    final maximumSeconds = min(
      _maximumRetryDelay.inSeconds,
      _baseRetryDelay.inSeconds * factor,
    );
    final half = maximumSeconds ~/ 2;
    return half + (jitter() * (maximumSeconds - half)).floor();
  }

  Future<void> _removeAccepted(BuzzPushLeaseRevocationRecord attempted) =>
      _serialize(() async {
        final records = await storage.loadAll();
        records.removeWhere(
          (record) =>
              record.leaseAddress == attempted.leaseAddress &&
              record.generation == attempted.generation + 1,
        );
        await storage.replaceAll(records);
      });

  void _scheduleNextWake() {
    if (!_started || _disposed) return;
    final generation = ++_wakeGeneration;
    _cancelWake?.call();
    _cancelWake = null;
    unawaited(
      _serialize(() async {
        final currentSeconds = now().millisecondsSinceEpoch ~/ 1000;
        final records = await storage.loadAll();
        final active = records
            .where(
              (record) =>
                  record.expiresAt > currentSeconds ||
                  record.gatewayRevocation != null,
            )
            .toList();
        if (active.length != records.length) {
          await storage.replaceAll(active);
        }
        if (active.isEmpty || generation != _wakeGeneration || _disposed) {
          return;
        }
        final wakeAt = active
            .map(
              (record) => record.gatewayRevocation == null
                  ? min(record.nextAttemptAt, record.expiresAt)
                  : record.nextAttemptAt,
            )
            .reduce(min);
        final delay = Duration(seconds: max(0, wakeAt - currentSeconds));
        _cancelWake = scheduleWake(delay, () {
          if (generation != _wakeGeneration || _disposed) return;
          _cancelWake = null;
          unawaited(trigger());
        });
      }),
    );
  }

  void dispose() {
    _disposed = true;
    _wakeGeneration += 1;
    _cancelWake?.call();
    _cancelWake = null;
  }
}

void Function() _scheduleTimer(Duration delay, void Function() wake) {
  final timer = Timer(delay, wake);
  return timer.cancel;
}

String canonicalBuzzPushRelayOrigin(String relayUrl) {
  final uri = _buzzPushRelayUri(relayUrl);
  final scheme = switch (uri.scheme) {
    'https' || 'wss' => 'wss',
    'http' || 'ws' => 'ws',
    _ => throw StateError('Validated relay URL has an unsupported scheme.'),
  };
  return '$scheme://${uri.authority}';
}

String canonicalBuzzPushRelayHttpUrl(String relayUrl) {
  final uri = _buzzPushRelayUri(relayUrl);
  final scheme = switch (uri.scheme) {
    'https' || 'wss' => 'https',
    'http' || 'ws' => 'http',
    _ => throw StateError('Validated relay URL has an unsupported scheme.'),
  };
  return uri.replace(scheme: scheme, path: '/').toString();
}

Uri _buzzPushRelayUri(String relayUrl) {
  final uri = Uri.tryParse(relayUrl);
  if (uri == null ||
      !const {'http', 'https', 'ws', 'wss'}.contains(uri.scheme) ||
      uri.host.isEmpty ||
      uri.userInfo.isNotEmpty ||
      (uri.path.isNotEmpty && uri.path != '/') ||
      uri.hasQuery ||
      uri.hasFragment) {
    throw FormatException('Invalid relay URL for push revocation: $relayUrl');
  }
  return uri;
}

Future<void> publishBuzzPushLeaseRevocation(
  BuzzPushLeaseRevocationRecord record, {
  bool revokeGateway = true,
  Map<String, dynamic>? gatewayRevocation,
}) async {
  if (revokeGateway && record.gatewayRevocation != null) {
    final target = gatewayRevocation ?? record.gatewayRevocation!;
    await revokeBuzzPushDelegation(target);
  }
  if (record.gatewayOnly ||
      record.expiresAt <= DateTime.now().millisecondsSinceEpoch ~/ 1000) {
    return;
  }
  final descriptor = await fetchBuzzPushLeaseDescriptor(record.relayUrl);
  if (descriptor.origin != record.relayOrigin) {
    throw StateError('Relay push origin changed while revocation was pending.');
  }
  final uri = Uri.parse(record.relayUrl);
  final wsUrl = uri
      .replace(scheme: uri.scheme == 'https' ? 'wss' : 'ws')
      .toString();
  await publishBuzzPushLeaseTombstone(
    descriptor: descriptor,
    installationId: record.installationId,
    generation: record.generation,
    nsec: record.nsec,
    memberPubkey: record.memberPubkey,
    submit: ({required kind, required content, required tags, createdAt}) =>
        submitSignedEventOnce(
          wsUrl: wsUrl,
          nsec: record.nsec,
          kind: kind,
          content: content,
          tags: tags,
          createdAt: createdAt,
        ),
  );
}

final buzzPushLeaseRevocationStorageProvider =
    Provider<BuzzPushLeaseRevocationStorage>(
      (ref) => BuzzPushLeaseRevocationStorage(),
    );

final buzzPushLeaseRevocationOutboxProvider =
    Provider<BuzzPushLeaseRevocationOutbox>((ref) {
      final outbox = BuzzPushLeaseRevocationOutbox(
        storage: ref.read(buzzPushLeaseRevocationStorageProvider),
        publisher: (record) async {
          var target = record.gatewayRevocation;
          var revokeGateway = target != null;
          if (target != null) {
            final grants = await readBuzzPushEndpointGrants();
            if (target['relayPubkey'] == '') {
              final relayOrigin = target['relayOrigin'];
              final known = grants.where(
                (grant) => grant.relayOrigin == relayOrigin,
              );
              if (known.length == 1) {
                target = known.single.gatewayRevocationTarget ?? target;
              }
            }
            final originalTarget = target;
            final communities =
                ref.read(communityListProvider).value ??
                await ref.read(communityStorageProvider).loadAll();
            // Recheck at execution: another community may have opted in since
            // this cleanup was journaled. Sharing is installation-scoped.
            revokeGateway =
                originalTarget['relayPubkey'] == '' ||
                !communities
                    .where((community) => community.pushNotificationsEnabled)
                    .any((community) {
                      final matching = grants.where(
                        (grant) =>
                            grant.relayOrigin ==
                            canonicalBuzzPushRelayOrigin(community.relayUrl),
                      );
                      return matching.any(
                        (grant) =>
                            grant.relayPubkey ==
                                originalTarget['relayPubkey'] &&
                            (originalTarget.containsKey('installationHandle')
                                ? grant.gatewayInstallationHandle ==
                                      originalTarget['installationHandle']
                                : grant.relayOrigin ==
                                      originalTarget['relayOrigin']),
                      );
                    });
          }
          await publishBuzzPushLeaseRevocation(
            record,
            revokeGateway: revokeGateway,
            gatewayRevocation: target,
          );
        },
      );
      ref.onDispose(outbox.dispose);
      return outbox;
    });
