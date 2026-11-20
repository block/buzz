import 'dart:async';
import 'dart:convert';
import 'dart:math';
import 'dart:typed_data';

import 'package:crypto/crypto.dart' as crypto;
import 'package:flutter_secure_storage/flutter_secure_storage.dart';

import '../storage/secure_storage.dart';
import 'android_push_gateway.dart';
import 'android_push_keystore.dart';
import 'android_push_transcript.dart';

/// Lifetime of an installation and of each delegation, as on iOS.
const _lifetimeSeconds = 2592000;

/// Backdates `not_before` and `expires_at` so a phone clock slightly ahead of
/// the gateway's is not refused.
const _clockSkewAllowanceSeconds = 120;

/// A grant in its final minutes is replaced by a new delegation.
const _renewWindowSeconds = 300;

/// The most a first enrollment waits, so phones that update or sign in
/// together do not all enroll in the same second.
const androidPushFirstEnrollmentJitterMax = Duration(seconds: 20);

/// The phone's push authority for one relay: the opaque grant the relay
/// stores in its lease, and the lease's installation id.
class AndroidPushGrant {
  const AndroidPushGrant({
    required this.relayOrigin,
    required this.relayPubkey,
    required this.installationId,
    required this.endpointGrant,
    required this.endpointHash,
    required this.endpointEpoch,
    required this.generation,
    required this.expiresAt,
  });

  final String relayOrigin;
  final String relayPubkey;
  final String installationId;
  final String endpointGrant;
  final String endpointHash;
  final int endpointEpoch;
  final int generation;
  final int expiresAt;
}

/// Durable authority state, as one secure-storage string.
abstract interface class AndroidPushStateStore {
  Future<String?> read();
  Future<void> write(String? value);
}

class SecureAndroidPushStateStore implements AndroidPushStateStore {
  SecureAndroidPushStateStore({FlutterSecureStorage? secure})
    : _secure = secure ?? communitySecureStorage;

  static const _key = 'buzz_android_push_authority_v1';

  final FlutterSecureStorage _secure;

  @override
  Future<String?> read() => _secure.read(key: _key);

  @override
  Future<void> write(String? value) => value == null
      ? _secure.delete(key: _key)
      : _secure.write(key: _key, value: value);
}

/// The Android installation and its per-relay delegations.
///
/// The Keystore key is the installation's only authority: it signs the
/// enrollment attestation challenge and every later call. Each call is
/// journaled before its first network attempt (the exact enrollment request;
/// the next delegation generation; an unfinished endpoint rotation), so a
/// lost response or a killed process is resumed rather than repeated with
/// different bytes. Calls that change state run one at a time.
class AndroidPushAuthority {
  AndroidPushAuthority({
    required this.keystore,
    required this.gateway,
    required this.store,
    DateTime Function()? now,
    Random? random,
    Future<void> Function(Duration delay)? sleep,
    this.firstEnrollmentJitterMax = androidPushFirstEnrollmentJitterMax,
  }) : _now = now ?? DateTime.now,
       _random = random ?? Random.secure(),
       _sleep = sleep ?? Future<void>.delayed;

  final AndroidPushKeystore keystore;
  final AndroidPushGatewayClient gateway;
  final AndroidPushStateStore store;
  final Duration firstEnrollmentJitterMax;
  final DateTime Function() _now;
  final Random _random;
  final Future<void> Function(Duration delay) _sleep;

  Future<void> _tail = Future.value();

  int get _nowSeconds => _now().millisecondsSinceEpoch ~/ 1000;

  Future<T> _serialized<T>(Future<T> Function() operation) {
    final result = _tail.then((_) => operation());
    _tail = result.then<void>((_) {}, onError: (Object _, StackTrace _) {});
    return result;
  }

  /// Every relay's current grant. A plain read of the stored state, outside
  /// the call queue: it must not wait behind gateway calls or the enrollment
  /// jitter, and the state is only ever replaced whole.
  Future<List<AndroidPushGrant>> grants() async {
    final state = await _load();
    return [for (final relay in state.relays.values) ?_grantOf(relay)];
  }

  /// Makes [endpoint] (the FCM token) the installation's endpoint and returns
  /// a grant from the installation to [relayPubkey] at [relayOrigin],
  /// enrolling, rotating and delegating only as far as the stored state needs.
  Future<AndroidPushGrant> enroll({
    required String relayOrigin,
    required String relayPubkey,
    required String endpoint,
  }) => _serialized(() async {
    var attemptedReset = false;
    while (true) {
      await _ensureInstallation(endpoint);
      final state = await _load();
      final relay = state.relays[relayOrigin];
      final reusable = relay == null
          ? null
          : _reusableGrant(state, relay, relayPubkey);
      if (reusable != null) return reusable;
      try {
        return await _delegate(relayOrigin, relayPubkey);
      } on AndroidPushGatewayRejection catch (rejection) {
        // The gateway answers every refused installation call with the same
        // 404. After the one case we can explain (an endpoint rotation whose
        // response was lost, which _rotate has already accounted for), a
        // refused delegation means the installation is gone: revoked,
        // expired, or enrolled under a key this phone no longer has.
        if (!rejection.isNotAuthorized || attemptedReset) rethrow;
        attemptedReset = true;
        await _reset();
      }
    }
  });

  /// Revokes the delegation of every relay [desiredRelays] no longer lists,
  /// and, when [revokeInstallation], the installation itself, which frees the
  /// FCM token and deletes the key. [desiredRelays] is read when the work
  /// runs, so a relay enabled while this waited is kept.
  Future<void> reconcile({
    required Set<String> Function() desiredRelays,
    required bool Function() revokeInstallation,
  }) => _serialized(() async {
    var state = await _load();
    final pending = state.pendingEnrollment;
    if (state.installation == null && pending != null && revokeInstallation()) {
      // An enrollment whose response was lost may have created an
      // installation that holds the FCM token. Learn its handle by replaying
      // the journaled request, so it can be revoked below.
      await _settlePendingEnrollment(pending);
      state = await _load();
    }
    if (state.installation == null) return;
    final desired = desiredRelays();
    for (final relay in state.relays.values.toList()) {
      if (relay.delegated && !desired.contains(relay.origin)) {
        await _revokeDelegation(relay);
      }
    }
    if (revokeInstallation()) await _revokeInstallation();
  });

  // ---- installation ----

  Future<void> _ensureInstallation(String endpoint) async {
    var state = await _load();
    if (state.pendingEnrollment != null) {
      await _finishEnrollment(state.pendingEnrollment!);
      state = await _load();
    }
    final installation = state.installation;
    if (installation != null) {
      final key = await keystore.current();
      if (key == null ||
          key.keyId != installation.keyId ||
          installation.expiresAt <= _nowSeconds) {
        // Nothing can sign for this installation any more, or the gateway no
        // longer holds it.
        await _reset();
      }
    }
    state = await _load();
    if (state.installation == null) {
      await _enrollFresh(endpoint);
      return;
    }
    var current = state.installation!;
    // A journaled rotation target comes first: it may have reached the
    // gateway, and the epoch it consumed must be accounted for before the
    // current token's rotation.
    while (current.endpoint != endpoint || current.rotatingTo != null) {
      current = await _rotate(current, current.rotatingTo ?? endpoint);
    }
  }

  Future<void> _enrollFresh(String endpoint) async {
    if ((await _load()).relays.isEmpty &&
        firstEnrollmentJitterMax > Duration.zero) {
      await _sleep(
        Duration(
          milliseconds: _random.nextInt(
            firstEnrollmentJitterMax.inMilliseconds,
          ),
        ),
      );
    }
    final challenge = await _challenge();
    final expiresAt =
        _nowSeconds - _clockSkewAllowanceSeconds + _lifetimeSeconds;
    final transcript = androidEnrollTranscript(
      challengeId: challenge.id,
      challenge: challenge.text,
      endpoint: endpoint,
      endpointEpoch: 1,
      expiresAt: expiresAt,
    );
    // The attestation challenge is the digest of exactly what the gateway
    // will rebuild from the request.
    final key = await keystore.generate(
      Uint8List.fromList(crypto.sha256.convert(transcript).bytes),
    );
    final pending = _PendingEnrollment(
      keyId: key.keyId,
      expiresAt: expiresAt,
      body: {
        'v': 1,
        'challenge_id': challenge.id,
        'challenge': challenge.text,
        'android_attestation': key.chainBase64,
        'endpoint': endpoint,
        'endpoint_epoch': 1,
        'expires_at': expiresAt,
      },
    );
    // Durable before the first network attempt: a lost response is resumed
    // with these exact bytes, which the gateway accepts again.
    await _save((await _load()).copyWith(pendingEnrollment: pending));
    await _finishEnrollment(pending);
  }

  Future<void> _finishEnrollment(_PendingEnrollment pending) async {
    if (pending.expiresAt <= _nowSeconds) {
      await _discardPendingEnrollment();
      return _enrollFresh(pending.body['endpoint']! as String);
    }
    final Map<String, Object?> response;
    try {
      response = await gateway.post(
        '/v1/installations',
        pending.body,
        expectedStatus: 201,
      );
    } on AndroidPushGatewayRejection catch (rejection) {
      if (rejection.isTransient) rethrow;
      await _discardPendingEnrollment();
      if (rejection.isInvalidAttestation) {
        throw const AndroidPushAttestationUnavailable(
          'the gateway did not accept this phone\'s key attestation',
        );
      }
      rethrow;
    }
    final handle = response['installation_handle'];
    final expiresAt = response['expires_at'];
    if (handle is! String || expiresAt is! int) {
      throw const FormatException(
        'Push gateway returned a malformed installation.',
      );
    }
    await _save(
      (await _load()).copyWith(
        clearPendingEnrollment: true,
        installation: _Installation(
          handle: handle,
          keyId: pending.keyId,
          endpoint: pending.body['endpoint']! as String,
          endpointEpoch: 1,
          expiresAt: expiresAt,
        ),
      ),
    );
  }

  /// Sign-out's view of a journaled enrollment: replay it so a created
  /// installation can be revoked. Nothing is enrolled afresh, and a request
  /// the gateway refuses or that expired created nothing.
  Future<void> _settlePendingEnrollment(_PendingEnrollment pending) async {
    if (pending.expiresAt <= _nowSeconds) {
      await _discardPendingEnrollment();
      return;
    }
    try {
      await _finishEnrollment(pending);
    } on AndroidPushAttestationUnavailable {
      // The gateway refused the attestation, so there is nothing to revoke.
    } on AndroidPushGatewayRejection catch (rejection) {
      if (rejection.isTransient) rethrow;
    }
  }

  Future<void> _discardPendingEnrollment() async {
    await keystore.delete();
    await _save((await _load()).copyWith(clearPendingEnrollment: true));
  }

  /// Local state for an installation the gateway no longer honors. Its
  /// delegations died with it, but their records stay readable: a lease
  /// tombstone still needs the relay's installation id.
  Future<void> _reset() async {
    await keystore.delete();
    final state = await _load();
    await _save(
      state.copyWith(
        clearInstallation: true,
        relays: {
          for (final relay in state.relays.values)
            relay.origin: relay.copyWith(generation: 0, delegated: false),
        },
      ),
    );
  }

  Future<_Installation> _rotate(
    _Installation installation,
    String endpoint,
  ) async {
    // Journal the target first: if the response is lost the gateway may be at
    // the next epoch already.
    await _save(
      (await _load()).copyWith(
        installation: installation.copyWith(rotatingTo: endpoint),
      ),
    );
    try {
      await _signed(
        '/v1/installations/endpoint',
        {
          'installation_handle': installation.handle,
          'endpoint_epoch': installation.endpointEpoch,
          'new_endpoint_epoch': installation.endpointEpoch + 1,
          'endpoint': endpoint,
        },
        (challenge) => androidRotateEndpointTranscript(
          challengeId: challenge.id,
          challenge: challenge.text,
          installationHandle: installation.handle,
          endpointEpoch: installation.endpointEpoch,
          newEndpointEpoch: installation.endpointEpoch + 1,
          endpoint: endpoint,
        ),
      );
    } on AndroidPushGatewayRejection catch (rejection) {
      // A retry of a rotation that already landed is refused like any other
      // bad call. Carry on as if it landed: the delegation that follows is at
      // the new epoch, so it succeeds exactly when that guess was right and
      // otherwise sends enroll() to a fresh installation.
      if (!rejection.isNotAuthorized) rethrow;
    }
    // Grants at the old epoch are void at the gateway. They stay stored, and
    // stop being reused because their epoch no longer matches.
    final rotated = installation.copyWith(
      endpoint: endpoint,
      endpointEpoch: installation.endpointEpoch + 1,
      clearRotatingTo: true,
    );
    await _save((await _load()).copyWith(installation: rotated));
    return rotated;
  }

  // ---- delegations ----

  AndroidPushGrant? _reusableGrant(
    _State state,
    _Relay relay,
    String relayPubkey,
  ) {
    final installation = state.installation!;
    final grant = relay.grant;
    if (grant == null ||
        !relay.delegated ||
        grant.installationHandle != installation.handle ||
        grant.relayPubkey != relayPubkey ||
        grant.endpointEpoch != installation.endpointEpoch ||
        grant.expiresAt <= _nowSeconds + _renewWindowSeconds) {
      return null;
    }
    return _grantOf(relay);
  }

  AndroidPushGrant? _grantOf(_Relay relay) {
    final grant = relay.grant;
    if (grant == null) return null;
    return AndroidPushGrant(
      relayOrigin: relay.origin,
      relayPubkey: grant.relayPubkey,
      installationId: relay.installationId,
      endpointGrant: grant.endpointGrant,
      endpointHash: grant.endpointHash,
      endpointEpoch: grant.endpointEpoch,
      generation: grant.generation,
      expiresAt: grant.expiresAt,
    );
  }

  Future<AndroidPushGrant> _delegate(
    String relayOrigin,
    String relayPubkey,
  ) async {
    var state = await _load();
    final installation = state.installation!;
    final existing = state.relays[relayOrigin];
    // The generation is spent before the call: a response that never arrived
    // may have been accepted, and the gateway needs each one to be higher.
    final generation = (existing?.generation ?? 0) + 1;
    final relay =
        (existing ??
                _Relay(
                  origin: relayOrigin,
                  installationId: _newInstallationId(),
                ))
            .copyWith(
              generation: generation,
              delegated: true,
              lastRelayPubkey: relayPubkey,
            );
    await _save(state.copyWith(relays: {...state.relays, relayOrigin: relay}));
    final notBefore = _nowSeconds - _clockSkewAllowanceSeconds;
    final expiresAt = notBefore + _lifetimeSeconds;
    final response = await _signed(
      '/v1/delegations',
      {
        'installation_handle': installation.handle,
        'endpoint_epoch': installation.endpointEpoch,
        'generation': generation,
        'relay_pubkey': relayPubkey,
        'not_before': notBefore,
        'expires_at': expiresAt,
      },
      (challenge) => androidDelegateTranscript(
        challengeId: challenge.id,
        challenge: challenge.text,
        installationHandle: installation.handle,
        endpointEpoch: installation.endpointEpoch,
        generation: generation,
        relayPubkey: relayPubkey,
        notBefore: notBefore,
        expiresAt: expiresAt,
      ),
      expectedStatus: 201,
    );
    final endpointGrant = response['endpoint_grant'];
    if (endpointGrant is! String || endpointGrant.isEmpty) {
      throw const FormatException('Push gateway returned a malformed grant.');
    }
    state = await _load();
    final granted = relay.copyWith(
      grant: _RelayGrant(
        installationHandle: installation.handle,
        relayPubkey: relayPubkey,
        endpointGrant: endpointGrant,
        endpointHash: crypto.sha256
            .convert(utf8.encode(installation.endpoint))
            .toString(),
        endpointEpoch: installation.endpointEpoch,
        generation: generation,
        expiresAt: expiresAt,
      ),
    );
    await _save(
      state.copyWith(
        relays: {...state.relays, relayOrigin: granted},
        installation: state.installation!.copyWith(
          // A delegation extends the installation to the grant's expiry.
          expiresAt: max(state.installation!.expiresAt, expiresAt),
        ),
      ),
    );
    return _grantOf(granted)!;
  }

  Future<void> _revokeDelegation(_Relay relay) async {
    final installation = (await _load()).installation!;
    final pubkey = relay.lastRelayPubkey;
    if (pubkey != null) {
      // The highest generation spent may never have been accepted, and the
      // gateway revokes by the generation it holds: try that one, then the
      // last one accepted.
      for (final generation in {relay.generation, ?relay.grant?.generation}) {
        try {
          await _signed(
            '/v1/delegations/revoke',
            {
              'installation_handle': installation.handle,
              'relay_pubkey': pubkey,
              'generation': generation,
            },
            (challenge) => androidRevokeDelegationTranscript(
              challengeId: challenge.id,
              challenge: challenge.text,
              installationHandle: installation.handle,
              relayPubkey: pubkey,
              generation: generation,
            ),
          );
          break;
        } on AndroidPushGatewayRejection catch (rejection) {
          // Not found at this generation. If no generation matches, the
          // delegation is already gone or its installation is, and there is
          // nothing left to revoke.
          if (!rejection.isNotAuthorized) rethrow;
        }
      }
    }
    final state = await _load();
    await _save(
      state.copyWith(
        relays: {
          ...state.relays,
          relay.origin: relay.copyWith(delegated: false),
        },
      ),
    );
  }

  Future<void> _revokeInstallation() async {
    final installation = (await _load()).installation!;
    // A journaled rotation may have reached the gateway, which then sits one
    // epoch ahead of what is stored. Try the stored epoch, then the next. The
    // key stays until a revocation lands or both are refused.
    final epochs = [
      installation.endpointEpoch,
      if (installation.rotatingTo != null) installation.endpointEpoch + 1,
    ];
    for (final epoch in epochs) {
      try {
        await _signed(
          '/v1/installations/revoke',
          {
            'installation_handle': installation.handle,
            'endpoint_epoch': epoch,
            'new_endpoint_epoch': epoch + 1,
          },
          (challenge) => androidRevokeInstallationTranscript(
            challengeId: challenge.id,
            challenge: challenge.text,
            installationHandle: installation.handle,
            endpointEpoch: epoch,
            newEndpointEpoch: epoch + 1,
          ),
        );
        break;
      } on AndroidPushGatewayRejection catch (rejection) {
        // Revoking twice succeeds, so a refusal at the last epoch means the
        // installation is already gone, which is what this call is for.
        if (!rejection.isNotAuthorized) rethrow;
      }
    }
    await keystore.delete();
    final state = await _load();
    await _save(
      state.copyWith(
        clearInstallation: true,
        relays: {
          for (final relay in state.relays.values)
            relay.origin: relay.copyWith(generation: 0, delegated: false),
        },
      ),
    );
  }

  // ---- gateway ----

  Future<_Challenge> _challenge() async {
    final response = await gateway.post('/v1/installations/challenges', {
      'v': 1,
    });
    final id = response['challenge_id'];
    final text = response['challenge'];
    if (id is! String || text is! String) {
      throw const FormatException(
        'Push gateway returned a malformed challenge.',
      );
    }
    return _Challenge(id, text);
  }

  Future<Map<String, Object?>> _signed(
    String route,
    Map<String, Object?> fields,
    Uint8List Function(_Challenge challenge) transcript, {
    int expectedStatus = 200,
  }) async {
    final challenge = await _challenge();
    final assertion = await keystore.sign(transcript(challenge));
    return gateway.post(route, {
      'v': 1,
      'challenge_id': challenge.id,
      'challenge': challenge.text,
      ...fields,
      'assertion': assertion,
    }, expectedStatus: expectedStatus);
  }

  String _newInstallationId() => [
    for (var i = 0; i < 16; i++)
      _random.nextInt(256).toRadixString(16).padLeft(2, '0'),
  ].join();

  // ---- storage ----

  Future<_State> _load() async {
    final raw = await store.read();
    if (raw == null) return _State.empty;
    final decoded = jsonDecode(raw);
    if (decoded is! Map<String, Object?>) {
      throw const FormatException('Android push authority state is malformed.');
    }
    return _State.fromJson(decoded);
  }

  Future<void> _save(_State state) =>
      store.write(state.isEmpty ? null : jsonEncode(state.toJson()));
}

class _Challenge {
  const _Challenge(this.id, this.text);

  final String id;
  final String text;
}

class _PendingEnrollment {
  const _PendingEnrollment({
    required this.keyId,
    required this.expiresAt,
    required this.body,
  });

  final String keyId;
  final int expiresAt;
  final Map<String, Object?> body;

  Map<String, Object?> toJson() => {
    'keyId': keyId,
    'expiresAt': expiresAt,
    'body': body,
  };

  factory _PendingEnrollment.fromJson(Map<String, Object?> json) =>
      _PendingEnrollment(
        keyId: json['keyId']! as String,
        expiresAt: json['expiresAt']! as int,
        body: Map<String, Object?>.from(json['body']! as Map),
      );
}

class _Installation {
  const _Installation({
    required this.handle,
    required this.keyId,
    required this.endpoint,
    required this.endpointEpoch,
    required this.expiresAt,
    this.rotatingTo,
  });

  final String handle;
  final String keyId;
  final String endpoint;
  final int endpointEpoch;
  final int expiresAt;

  /// The endpoint of a rotation that may have reached the gateway.
  final String? rotatingTo;

  _Installation copyWith({
    String? endpoint,
    int? endpointEpoch,
    int? expiresAt,
    String? rotatingTo,
    bool clearRotatingTo = false,
  }) => _Installation(
    handle: handle,
    keyId: keyId,
    endpoint: endpoint ?? this.endpoint,
    endpointEpoch: endpointEpoch ?? this.endpointEpoch,
    expiresAt: expiresAt ?? this.expiresAt,
    rotatingTo: clearRotatingTo ? null : rotatingTo ?? this.rotatingTo,
  );

  Map<String, Object?> toJson() => {
    'handle': handle,
    'keyId': keyId,
    'endpoint': endpoint,
    'endpointEpoch': endpointEpoch,
    'expiresAt': expiresAt,
    'rotatingTo': rotatingTo,
  };

  factory _Installation.fromJson(Map<String, Object?> json) => _Installation(
    handle: json['handle']! as String,
    keyId: json['keyId']! as String,
    endpoint: json['endpoint']! as String,
    endpointEpoch: json['endpointEpoch']! as int,
    expiresAt: json['expiresAt']! as int,
    rotatingTo: json['rotatingTo'] as String?,
  );
}

class _RelayGrant {
  const _RelayGrant({
    required this.installationHandle,
    required this.relayPubkey,
    required this.endpointGrant,
    required this.endpointHash,
    required this.endpointEpoch,
    required this.generation,
    required this.expiresAt,
  });

  final String installationHandle;
  final String relayPubkey;
  final String endpointGrant;
  final String endpointHash;
  final int endpointEpoch;
  final int generation;
  final int expiresAt;

  Map<String, Object?> toJson() => {
    'installationHandle': installationHandle,
    'relayPubkey': relayPubkey,
    'endpointGrant': endpointGrant,
    'endpointHash': endpointHash,
    'endpointEpoch': endpointEpoch,
    'generation': generation,
    'expiresAt': expiresAt,
  };

  factory _RelayGrant.fromJson(Map<String, Object?> json) => _RelayGrant(
    installationHandle: json['installationHandle']! as String,
    relayPubkey: json['relayPubkey']! as String,
    endpointGrant: json['endpointGrant']! as String,
    endpointHash: json['endpointHash']! as String,
    endpointEpoch: json['endpointEpoch']! as int,
    generation: json['generation']! as int,
    expiresAt: json['expiresAt']! as int,
  );
}

/// What the phone holds for one relay origin.
class _Relay {
  const _Relay({
    required this.origin,
    required this.installationId,
    this.generation = 0,
    this.delegated = false,
    this.grant,
    this.lastRelayPubkey,
  });

  final String origin;

  /// The lease's identity, kept across installations.
  final String installationId;

  /// The highest delegation generation spent, accepted or not.
  final int generation;

  /// Whether a delegation from this installation to the relay may exist at
  /// the gateway: set before the first attempt, cleared by revocation.
  final bool delegated;

  /// The last delegation the gateway accepted. It stays after revocation or
  /// a reset, because a lease tombstone still reads it.
  final _RelayGrant? grant;

  /// The relay key of the delegation attempted last, for revoking one whose
  /// response was lost.
  final String? lastRelayPubkey;

  _Relay copyWith({
    int? generation,
    bool? delegated,
    _RelayGrant? grant,
    String? lastRelayPubkey,
  }) => _Relay(
    origin: origin,
    installationId: installationId,
    generation: generation ?? this.generation,
    delegated: delegated ?? this.delegated,
    grant: grant ?? this.grant,
    lastRelayPubkey: lastRelayPubkey ?? this.lastRelayPubkey,
  );

  Map<String, Object?> toJson() => {
    'origin': origin,
    'installationId': installationId,
    'generation': generation,
    'delegated': delegated,
    'grant': grant?.toJson(),
    'lastRelayPubkey': lastRelayPubkey,
  };

  factory _Relay.fromJson(Map<String, Object?> json) => _Relay(
    origin: json['origin']! as String,
    installationId: json['installationId']! as String,
    generation: json['generation']! as int,
    delegated: json['delegated']! as bool,
    grant: json['grant'] == null
        ? null
        : _RelayGrant.fromJson(json['grant']! as Map<String, Object?>),
    lastRelayPubkey: json['lastRelayPubkey'] as String?,
  );
}

class _State {
  const _State({
    this.installation,
    this.pendingEnrollment,
    this.relays = const {},
  });

  static const empty = _State();

  final _Installation? installation;
  final _PendingEnrollment? pendingEnrollment;
  final Map<String, _Relay> relays;

  bool get isEmpty =>
      installation == null && pendingEnrollment == null && relays.isEmpty;

  _State copyWith({
    _Installation? installation,
    bool clearInstallation = false,
    _PendingEnrollment? pendingEnrollment,
    bool clearPendingEnrollment = false,
    Map<String, _Relay>? relays,
  }) => _State(
    installation: clearInstallation ? null : installation ?? this.installation,
    pendingEnrollment: clearPendingEnrollment
        ? null
        : pendingEnrollment ?? this.pendingEnrollment,
    relays: relays ?? this.relays,
  );

  Map<String, Object?> toJson() => {
    'installation': installation?.toJson(),
    'pendingEnrollment': pendingEnrollment?.toJson(),
    'relays': [for (final relay in relays.values) relay.toJson()],
  };

  factory _State.fromJson(Map<String, Object?> json) => _State(
    installation: json['installation'] == null
        ? null
        : _Installation.fromJson(json['installation']! as Map<String, Object?>),
    pendingEnrollment: json['pendingEnrollment'] == null
        ? null
        : _PendingEnrollment.fromJson(
            json['pendingEnrollment']! as Map<String, Object?>,
          ),
    relays: {
      for (final value in json['relays']! as List<Object?>)
        (value! as Map<String, Object?>)['origin']! as String: _Relay.fromJson(
          value as Map<String, Object?>,
        ),
    },
  );
}
