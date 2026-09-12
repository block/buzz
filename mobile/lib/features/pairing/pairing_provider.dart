import 'dart:async';
import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/auth/auth.dart';
import '../../shared/crypto/ecdh.dart';
import '../../shared/crypto/nip44.dart';
import '../../shared/relay/relay.dart';
import '../../shared/security/sensitive_action_authorizer.dart';
import 'pairing_crypto.dart';
import 'pairing_socket.dart';

/// HTTP client used by [PairingNotifier] for the validation request.
final pairingHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

enum PairingStatus {
  idle,
  connecting,
  confirmingSas,
  transferring,
  storing,
  success,
  error,
}

class PairingState {
  final PairingStatus status;
  final String? errorMessage;
  final String? sasCode;
  final bool userConfirmedSas;
  final bool sendsIdentityToDesktop;
  final bool protectSensitiveActions;
  final bool authorizationInProgress;

  const PairingState({
    this.status = PairingStatus.idle,
    this.errorMessage,
    this.sasCode,
    this.userConfirmedSas = false,
    this.sendsIdentityToDesktop = false,
    this.protectSensitiveActions = true,
    this.authorizationInProgress = false,
  });

  PairingState copyWith({
    PairingStatus? status,
    String? errorMessage,
    String? sasCode,
    bool? userConfirmedSas,
    bool? sendsIdentityToDesktop,
    bool? protectSensitiveActions,
    bool? authorizationInProgress,
    bool clearErrorMessage = false,
  }) => PairingState(
    status: status ?? this.status,
    errorMessage: clearErrorMessage ? null : errorMessage ?? this.errorMessage,
    sasCode: sasCode ?? this.sasCode,
    userConfirmedSas: userConfirmedSas ?? this.userConfirmedSas,
    sendsIdentityToDesktop:
        sendsIdentityToDesktop ?? this.sendsIdentityToDesktop,
    protectSensitiveActions:
        protectSensitiveActions ?? this.protectSensitiveActions,
    authorizationInProgress:
        authorizationInProgress ?? this.authorizationInProgress,
  );
}

typedef PairingSocketFactory =
    PairingSocket Function({
      required String wsUrl,
      required String ephemeralPrivkey,
      required void Function(List<dynamic> message) onMessage,
      required void Function(Object? error) onDisconnected,
    });

typedef PairingCredentialValidator =
    Future<void> Function({required String relayUrl, required String? nsec});

const identityExportAuthorizationTtl = Duration(minutes: 2);

final identityExportClockProvider = Provider<DateTime Function()>((ref) {
  return DateTime.now;
});

class PairingNotifier extends Notifier<PairingState> {
  final PairingSocketFactory _socketFactory;
  final PairingCredentialValidator? _credentialValidator;
  PairingSocket? _socket;
  Timer? _sessionTimeout;
  Community? _identityExportCommunity;
  bool _identityExportBiometricOnly = false;

  PairingNotifier({
    PairingSocketFactory? socketFactory,
    PairingCredentialValidator? credentialValidator,
  }) : _socketFactory = socketFactory ?? _createPairingSocket,
       _credentialValidator = credentialValidator;

  static PairingSocket _createPairingSocket({
    required String wsUrl,
    required String ephemeralPrivkey,
    required void Function(List<dynamic> message) onMessage,
    required void Function(Object? error) onDisconnected,
  }) => PairingSocket(
    wsUrl: wsUrl,
    ephemeralPrivkey: ephemeralPrivkey,
    onMessage: onMessage,
    onDisconnected: onDisconnected,
  );

  @override
  PairingState build() => const PairingState();

  Future<void> pair(String rawInput) async {
    if (state.status == PairingStatus.connecting ||
        state.status == PairingStatus.confirmingSas ||
        state.status == PairingStatus.transferring) {
      return;
    }

    final trimmed = rawInput.trim();
    if (trimmed.startsWith('nostrpair://')) {
      return _pairNipAb(trimmed);
    }
    // Legacy buzz:// flow.
    return _pairLegacy(trimmed);
  }

  Future<bool> authorizeIdentityExport({required Community community}) async {
    if (state.authorizationInProgress) return false;

    final biometricOnly =
        community.sensitiveActionPolicy == SensitiveActionPolicy.enabled;
    final pairingGeneration = _pairingGeneration;
    state = state.copyWith(
      authorizationInProgress: true,
      clearErrorMessage: true,
    );
    final result = await ref
        .read(sensitiveActionAuthorizationSessionProvider)
        .authorize(biometricOnly: biometricOnly);
    if (pairingGeneration != _pairingGeneration) return false;
    if (result != DeviceAuthResult.success) {
      state = state.copyWith(
        authorizationInProgress: false,
        errorMessage: _authorizationError(result),
      );
      return false;
    }

    _identityExportCommunity = community;
    _identityExportBiometricOnly = biometricOnly;
    _identityExportAuthorizedAt = ref.read(identityExportClockProvider)();
    state = state.copyWith(authorizationInProgress: false);
    return true;
  }

  /// Confirm that the SAS code matches. Called by the UI after user approval.
  void confirmSas() {
    if (state.status != PairingStatus.confirmingSas ||
        state.authorizationInProgress) {
      return;
    }
    _userConfirmedSas = true;
    state = state.copyWith(userConfirmedSas: true);
    if (_sasConfirmReceived) unawaited(_continueAfterSas());
  }

  void setProtectSensitiveActions(bool value) {
    if (state.status != PairingStatus.confirmingSas ||
        state.sendsIdentityToDesktop ||
        state.authorizationInProgress) {
      return;
    }
    state = state.copyWith(protectSensitiveActions: value);
  }

  Future<void> _continueAfterSas() async {
    if (!_userConfirmedSas ||
        !_sasConfirmReceived ||
        state.status != PairingStatus.confirmingSas ||
        state.authorizationInProgress) {
      return;
    }

    if (_sendIdentityToSource && !_exportIdentityIsCurrent()) {
      _userConfirmedSas = false;
      state = state.copyWith(
        userConfirmedSas: false,
        errorMessage:
            'The active community changed. Start identity export again.',
      );
      return;
    }

    final authorizedAt = _identityExportAuthorizedAt;
    final elapsed = authorizedAt == null
        ? null
        : ref.read(identityExportClockProvider)().difference(authorizedAt);
    final hasFreshExportAuthorization =
        elapsed != null &&
        !elapsed.isNegative &&
        elapsed < identityExportAuthorizationTtl;
    if (_sendIdentityToSource && !hasFreshExportAuthorization) {
      final pairingGeneration = _pairingGeneration;
      state = state.copyWith(authorizationInProgress: true);
      final result = await ref
          .read(sensitiveActionAuthorizationSessionProvider)
          .authorize(biometricOnly: _identityExportBiometricOnly);
      if (pairingGeneration != _pairingGeneration ||
          !_userConfirmedSas ||
          !_sasConfirmReceived ||
          state.status != PairingStatus.confirmingSas ||
          !state.authorizationInProgress) {
        return;
      }
      if (result != DeviceAuthResult.success) {
        _userConfirmedSas = false;
        state = state.copyWith(
          userConfirmedSas: false,
          authorizationInProgress: false,
          errorMessage: _authorizationError(result),
        );
        return;
      }
    } else if (!_sendIdentityToSource && state.protectSensitiveActions) {
      final pairingGeneration = _pairingGeneration;
      state = state.copyWith(authorizationInProgress: true);
      final result = await ref
          .read(sensitiveActionAuthorizerProvider)
          .authorizeBiometricProtection();
      if (pairingGeneration != _pairingGeneration ||
          !_userConfirmedSas ||
          !_sasConfirmReceived ||
          state.status != PairingStatus.confirmingSas ||
          !state.authorizationInProgress) {
        return;
      }
      if (result != DeviceAuthResult.success) {
        _userConfirmedSas = false;
        state = state.copyWith(
          userConfirmedSas: false,
          authorizationInProgress: false,
          errorMessage: _biometricProtectionError(result),
        );
        return;
      }
    }

    _userConfirmedSas = false;
    state = state.copyWith(
      status: PairingStatus.transferring,
      authorizationInProgress: false,
    );
    if (_sendIdentityToSource) {
      _sendIdentityPayload();
    } else {
      final pending = _pendingPayload;
      if (pending != null) {
        _pendingPayload = null;
        _handlePayload(pending);
      }
    }
  }

  static String _biometricProtectionError(
    DeviceAuthResult result,
  ) => switch (result) {
    DeviceAuthResult.cancelled =>
      'Biometric setup was cancelled. Nothing was transferred.',
    DeviceAuthResult.unavailable =>
      'Biometrics are unavailable. Enroll Face ID or biometrics and try again, or turn this option off.',
    DeviceAuthResult.lockedOut =>
      'Biometrics are locked. Unlock them in system settings and try again.',
    DeviceAuthResult.failed =>
      'Biometric confirmation failed. Nothing was transferred.',
    DeviceAuthResult.success => '',
  };

  static String _authorizationError(
    DeviceAuthResult result,
  ) => switch (result) {
    DeviceAuthResult.cancelled =>
      'Identity confirmation was cancelled. Nothing was transferred.',
    DeviceAuthResult.unavailable =>
      'Device authentication is unavailable. Configure a device passcode or biometrics and try again.',
    DeviceAuthResult.lockedOut =>
      'Device authentication is locked. Unlock it in system settings and try again.',
    DeviceAuthResult.failed =>
      'Identity confirmation failed. Nothing was transferred.',
    DeviceAuthResult.success => '',
  };

  /// Deny the SAS code. Send abort and terminate.
  void denySas() {
    _sendAbort('sas_mismatch');
    _cleanup();
    state = PairingState(
      status: PairingStatus.error,
      errorMessage: 'SAS code mismatch — pairing cancelled for security.',
    );
  }

  void reset() {
    _cleanup();
    state = const PairingState();
  }

  void _cleanup() {
    _pairingGeneration++;
    _sessionTimeout?.cancel();
    _sessionTimeout = null;
    _socket?.dispose();
    _socket = null;
    _processedEventIds.clear();
    _sasConfirmReceived = false;
    _userConfirmedSas = false;
    _pendingPayload = null;
    _sendIdentityToSource = false;
    _identityExportCommunity = null;
    _identityExportBiometricOnly = false;
    _identityExportAuthorizedAt = null;
  }

  // ── NIP-AB pairing flow ─────────────────────────────────────────────────

  // Session state kept between steps.
  String? _ephemeralPrivkey;
  String? _ephemeralPubkey;
  Uint8List? _sessionSecret;
  String? _sourcePubkey;
  Uint8List? _sessionId;
  Uint8List? _sasInput;
  Uint8List? _conversationKey;
  bool _sasConfirmReceived = false;
  bool _userConfirmedSas = false;
  bool _sendIdentityToSource = false;
  int _pairingGeneration = 0;
  DateTime? _identityExportAuthorizedAt;
  Map<String, dynamic>? _pendingPayload; // buffered until user confirms SAS
  final Set<String> _processedEventIds = {}; // NIP-AB §Duplicate Event Handling

  Future<void> _pairNipAb(String uri) async {
    state = const PairingState(status: PairingStatus.connecting);

    try {
      // 1. Parse the nostrpair:// URI.
      final qr = parseNostrpairUri(uri);
      _sourcePubkey = qr.sourcePubkey;
      _sessionSecret = qr.sessionSecret;
      _sendIdentityToSource =
          Uri.parse(uri).queryParameters['mode'] == 'recover';

      final relayWsUrl = qr.relays.first;

      // 2. Generate ephemeral keypair.
      final keychain = nostr.Keys.generate();
      _ephemeralPrivkey = keychain.secret;
      _ephemeralPubkey = keychain.public;

      // 3. Derive session ID and SAS immediately (we know source pubkey from QR).
      _sessionId = deriveSessionId(qr.sessionSecret);
      final ecdhShared = ecdhSharedSecret(_ephemeralPrivkey!, qr.sourcePubkey);
      final (sasCode, sasInput) = deriveSas(ecdhShared, qr.sessionSecret);
      _sasInput = sasInput;

      // Pre-compute NIP-44 conversation key for encrypting events.
      _conversationKey = getConversationKey(
        _ephemeralPrivkey!,
        qr.sourcePubkey,
      );

      // 4. Connect to relay with ephemeral keys.
      final socket = _socketFactory(
        wsUrl: relayWsUrl,
        ephemeralPrivkey: _ephemeralPrivkey!,
        onMessage: _handleRelayMessage,
        onDisconnected: _handleDisconnected,
      );
      _socket = socket;
      await socket.connect();

      if (!socket.isConnected) {
        throw StateError('Pairing socket did not reach the connected state');
      }

      // 5. Subscribe for kind:24134 events tagged to our ephemeral pubkey.
      socket.subscribe('pair', 24134, _ephemeralPubkey!);

      // 6. Wait briefly for EOSE, then send offer.
      // (In practice, we send the offer immediately — the relay will buffer it.)
      await Future.delayed(const Duration(milliseconds: 500));

      // 7. Build and send the offer event.
      final offerContent = _encryptMessage({
        'type': 'offer',
        'version': 1,
        'session_id': bytesToHex(_sessionId!),
      });

      _publishEvent(
        kind: 24134,
        content: offerContent,
        tags: [
          ['p', qr.sourcePubkey],
        ],
      );

      // 8. Display SAS code and wait for sas-confirm from source.
      state = PairingState(
        status: PairingStatus.confirmingSas,
        sasCode: formatSas(sasCode),
        sendsIdentityToDesktop: _sendIdentityToSource,
        protectSensitiveActions: ref.read(relayConfigProvider).nsec == null,
      );

      // 9. Start 120s session timeout.
      _sessionTimeout = Timer(const Duration(seconds: 120), () {
        if (state.status != PairingStatus.success &&
            state.status != PairingStatus.error) {
          _cleanup();
          state = const PairingState(
            status: PairingStatus.error,
            errorMessage: 'Pairing session timed out.',
          );
        }
      });
    } on FormatException catch (e) {
      _cleanup();
      state = PairingState(
        status: PairingStatus.error,
        errorMessage: 'Invalid pairing code: ${e.message}',
      );
    } catch (e) {
      debugPrint('Pairing connection error: $e');
      _cleanup();
      state = PairingState(
        status: PairingStatus.error,
        errorMessage: _friendlyErrorMessage(e),
      );
    }
  }

  static String _friendlyErrorMessage(Object error) {
    final message = error.toString();
    if (message.contains('SocketException') ||
        message.contains('Connection refused') ||
        message.contains('Network is unreachable') ||
        message.contains('No route to host') ||
        message.contains('Failed to connect')) {
      return 'Could not reach the pairing relay. Check your internet '
          'connection and VPN, then try again.';
    }
    if (error is PairingAuthException) {
      return 'The pairing relay rejected authentication. Try creating a new '
          'pairing code.';
    }
    if (error is StateError ||
        message.contains('Null check operator used on a null value')) {
      return 'Pairing stopped because of an internal error. Please try again.';
    }
    if (message.contains('HandshakeException') ||
        message.contains('CERTIFICATE_VERIFY_FAILED')) {
      return 'Secure connection failed. Check your network settings '
          'and try again.';
    }
    if (message.contains('TimeoutException') || message.contains('timed out')) {
      return 'Connection timed out. Check your internet connection and '
          'try again.';
    }
    return 'Connection failed. Please check your internet connection '
        'and try again.';
  }

  void _handleRelayMessage(List<dynamic> data) {
    if (data.isEmpty) return;
    final type = data[0] as String;

    if (type == 'EVENT' && data.length >= 3) {
      final eventJson = data[2] as Map<String, dynamic>;
      _handlePairingEvent(eventJson);
    }
    // Ignore EOSE, NOTICE, etc.
  }

  void _handlePairingEvent(Map<String, dynamic> eventJson) {
    try {
      // NIP-AB §Event Validation: validate kind.
      final kind = eventJson['kind'] as int?;
      if (kind != 24134) return;

      // NIP-AB §Event Validation: validate pubkey is from expected source.
      final eventPubkey = eventJson['pubkey'] as String?;
      if (eventPubkey == null) return;
      if (_sourcePubkey != null && eventPubkey != _sourcePubkey) return;

      // NIP-AB §Duplicate Event Handling: discard already-processed events.
      final eventId = eventJson['id'] as String?;
      if (eventId == null) return;
      if (_processedEventIds.contains(eventId)) return;

      // NIP-AB §Event Validation: check p-tag points to us.
      final tags = (eventJson['tags'] as List<dynamic>?) ?? [];
      final hasOurPTag = tags.any((t) {
        if (t is List && t.length >= 2) {
          return t[0] == 'p' && t[1] == _ephemeralPubkey;
        }
        return false;
      });
      if (!hasOurPTag) return;

      // NIP-AB §Event Validation: verify event signature (NIP-01).
      // The nostr package's Event.fromJson verifies id + sig on construction.
      try {
        final event = nostr.Event.fromJson(jsonEncode(eventJson));
        if (event.id != eventId) return; // id mismatch
      } catch (_) {
        return; // invalid signature or malformed event
      }

      // Decrypt NIP-44 content.
      final content = eventJson['content'] as String?;
      if (content == null || content.isEmpty) return;

      final decryptKey = getConversationKey(_ephemeralPrivkey!, eventPubkey);
      final decrypted = nip44Decrypt(decryptKey, content);
      final msg = jsonDecode(decrypted) as Map<String, dynamic>;
      final msgType = msg['type'] as String?;

      switch (msgType) {
        case 'sas-confirm':
          _handleSasConfirm(msg);
          _processedEventIds.add(eventId); // record after successful processing
        case 'payload':
          _handlePayload(msg);
          _processedEventIds.add(eventId);
        case 'abort':
          _handleAbort(msg);
          _processedEventIds.add(eventId);
        case 'complete':
          _handleComplete(msg);
          _processedEventIds.add(eventId);
      }
    } catch (e) {
      // Silently discard invalid events per NIP-AB §Event Validation.
    }
  }

  void _handleSasConfirm(Map<String, dynamic> msg) {
    if (state.status != PairingStatus.confirmingSas) return;

    final receivedHash = msg['transcript_hash'] as String?;
    if (receivedHash == null) return;

    // Verify transcript hash.
    final expectedHash = deriveTranscriptHash(
      _sessionId!,
      hexToBytes(_sourcePubkey!),
      hexToBytes(_ephemeralPubkey!),
      _sasInput!,
      _sessionSecret!,
    );

    final receivedBytes = hexToBytes(receivedHash);
    if (!constantTimeEquals(receivedBytes, expectedHash)) {
      // NIP-AB §Step 3: target MUST send abort with reason "sas_mismatch".
      _sendAbort('sas_mismatch');
      _cleanup();
      state = const PairingState(
        status: PairingStatus.error,
        errorMessage:
            'Security verification failed — possible attack. Pairing aborted.',
      );
      return;
    }

    _sasConfirmReceived = true;

    // If the user already tapped "Codes Match", complete the transition now
    // that the transcript hash is verified.
    if (_userConfirmedSas) {
      unawaited(_continueAfterSas());
    }
    // Otherwise stay in confirmingSas — user must still confirm via confirmSas().
  }

  bool _exportIdentityIsCurrent() {
    final authorizedCommunity = _identityExportCommunity;
    final currentConfig = ref.read(relayConfigProvider);
    return authorizedCommunity != null &&
        authorizedCommunity.nsec != null &&
        authorizedCommunity.nsec!.isNotEmpty &&
        authorizedCommunity.nsec == currentConfig.nsec &&
        authorizedCommunity.relayUrl == currentConfig.storedOrigin;
  }

  void _sendIdentityPayload() {
    if (!_exportIdentityIsCurrent()) {
      _sendAbort('identity_changed');
      _cleanup();
      state = const PairingState(
        status: PairingStatus.error,
        errorMessage:
            'The active community changed. Start identity export again.',
      );
      return;
    }
    final nsec = _identityExportCommunity!.nsec!;
    final content = _encryptMessage({
      'type': 'payload',
      'payload_type': 'nsec',
      'payload': nsec,
    });
    _publishEvent(
      kind: 24134,
      content: content,
      tags: [
        ['p', _sourcePubkey!],
      ],
    );
  }

  void _handlePayload(Map<String, dynamic> msg) {
    // Only accept payload after the transcript hash was verified.
    if (!_sasConfirmReceived) return;

    // If the user hasn't confirmed SAS yet, buffer the payload.
    // It will be processed when confirmSas() is called.
    if (state.status == PairingStatus.confirmingSas) {
      _pendingPayload = msg;
      return;
    }
    if (state.status != PairingStatus.transferring) return;

    state = state.copyWith(status: PairingStatus.storing);

    final payloadType = msg['payload_type'] as String?;
    final payload = msg['payload'] as String?;
    if (payload == null) {
      _cleanup();
      state = const PairingState(
        status: PairingStatus.error,
        errorMessage: 'Received empty payload from source.',
      );
      return;
    }

    final pairingGeneration = _pairingGeneration;
    final protectSensitiveActions = state.protectSensitiveActions;
    unawaited(
      _processPayload(
        payloadType,
        payload,
        pairingGeneration: pairingGeneration,
        protectSensitiveActions: protectSensitiveActions,
      ),
    );
  }

  void _handleComplete(Map<String, dynamic> msg) {
    if (!_sendIdentityToSource || state.status != PairingStatus.transferring) {
      return;
    }
    if (msg['success'] != true) {
      _cleanup();
      state = const PairingState(
        status: PairingStatus.error,
        errorMessage: 'Desktop could not store the identity.',
      );
      return;
    }
    _cleanup();
    state = const PairingState(status: PairingStatus.success);
  }

  void _handleAbort(Map<String, dynamic> msg) {
    final reason = msg['reason'] as String? ?? 'unknown';
    _cleanup();
    state = PairingState(
      status: PairingStatus.error,
      errorMessage: 'Source device aborted pairing: $reason',
    );
  }

  Future<void> _processPayload(
    String? payloadType,
    String payload, {
    required int pairingGeneration,
    required bool protectSensitiveActions,
  }) async {
    try {
      // Parse the custom payload.
      final data = jsonDecode(payload) as Map<String, dynamic>;
      final relayUrl = data['relayUrl'] as String?;
      final pubkey = data['pubkey'] as String?;
      final nsec = data['nsec'] as String?;

      if (relayUrl == null) {
        throw const FormatException('Missing relayUrl in payload');
      }

      // Validate relay URL to prevent SSRF via private network addresses.
      _validateRelayUrl(relayUrl);

      // Validate credentials against the relay via NIP-42 WS handshake.
      final credentialValidator = _credentialValidator ?? _validateCredentials;
      await credentialValidator(relayUrl: relayUrl, nsec: nsec);
      if (pairingGeneration != _pairingGeneration ||
          state.status != PairingStatus.storing ||
          _sendIdentityToSource) {
        return;
      }

      // Send complete only after credentials are validated.
      _sendComplete(true);

      // Store as community and switch to it.
      final community = Community.create(
        name: Community.nameFromUrl(relayUrl),
        relayUrl: relayUrl,
        pubkey: pubkey,
        nsec: nsec,
        sensitiveActionPolicy: protectSensitiveActions
            ? SensitiveActionPolicy.enabled
            : SensitiveActionPolicy.disabledByUser,
      );
      await ref
          .read(authProvider.notifier)
          .authenticateWithCommunity(community);
      if (pairingGeneration != _pairingGeneration ||
          state.status != PairingStatus.storing ||
          _sendIdentityToSource) {
        return;
      }

      _cleanup();
      state = const PairingState(status: PairingStatus.success);
    } catch (e) {
      if (pairingGeneration != _pairingGeneration ||
          state.status != PairingStatus.storing ||
          _sendIdentityToSource) {
        return;
      }
      _sendComplete(false);
      _cleanup();
      state = PairingState(
        status: PairingStatus.error,
        errorMessage: 'Failed to import credentials: $e',
      );
    }
  }

  void _sendAbort(String reason) {
    try {
      final content = _encryptMessage({'type': 'abort', 'reason': reason});
      _publishEvent(
        kind: 24134,
        content: content,
        tags: [
          ['p', _sourcePubkey!],
        ],
      );
    } catch (_) {
      // Best-effort.
    }
  }

  void _sendComplete(bool success) {
    try {
      final content = _encryptMessage({'type': 'complete', 'success': success});
      _publishEvent(
        kind: 24134,
        content: content,
        tags: [
          ['p', _sourcePubkey!],
        ],
      );
    } catch (_) {
      // Best-effort — complete is advisory per NIP-AB.
    }
  }

  /// Encrypt a message using NIP-44 with the ephemeral conversation key.
  String _encryptMessage(Map<String, dynamic> message) {
    final plaintext = jsonEncode(message);
    return nip44Encrypt(_conversationKey!, plaintext);
  }

  /// Build and publish a kind:24134 event signed with ephemeral keys.
  void _publishEvent({
    required int kind,
    required String content,
    required List<List<String>> tags,
  }) {
    // Add timestamp jitter (0-30s) for metadata privacy.
    final jitter = math.Random.secure().nextInt(31);
    final createdAt = (DateTime.now().millisecondsSinceEpoch ~/ 1000) - jitter;

    final event = nostr.Event.from(
      kind: kind,
      content: content,
      tags: tags,
      secretKey: _ephemeralPrivkey!,
      createdAt: createdAt,
    );

    _socket?.publishEvent(event.toMap());
  }

  void _handleDisconnected(Object? error) {
    if (state.status == PairingStatus.success ||
        state.status == PairingStatus.error) {
      return;
    }
    _cleanup();
    state = PairingState(
      status: PairingStatus.error,
      errorMessage: 'Lost connection to pairing relay.',
    );
  }

  // ── Legacy buzz:// flow ───────────────────────────────────────────────

  Future<void> _pairLegacy(String rawInput) async {
    state = const PairingState(status: PairingStatus.connecting);

    try {
      final community = _parseLegacyInput(rawInput);
      await _validateCredentials(
        relayUrl: community.relayUrl,
        nsec: community.nsec,
      );

      await ref
          .read(authProvider.notifier)
          .authenticateWithCommunity(community);
      state = const PairingState(status: PairingStatus.success);
    } on FormatException catch (e) {
      state = PairingState(
        status: PairingStatus.error,
        errorMessage: 'Invalid pairing code: ${e.message}',
      );
    } on RelayException catch (e) {
      state = PairingState(
        status: PairingStatus.error,
        errorMessage:
            'Could not connect to relay (${e.statusCode}). '
            'Check that the pairing code is valid.',
      );
    } catch (e) {
      state = PairingState(
        status: PairingStatus.error,
        errorMessage:
            'Connection failed. Make sure your device can reach the '
            'relay server.',
      );
    }
  }

  Future<void> _validateCredentials({
    required String relayUrl,
    required String? nsec,
  }) async {
    if (nsec == null || nsec.isEmpty) {
      throw const FormatException('Pairing payload missing nsec');
    }
    final uri = Uri.parse(relayUrl);
    final scheme = uri.scheme == 'https' ? 'wss' : 'ws';
    final wsUrl = uri.replace(scheme: scheme).toString();

    final socket = RelaySocket(
      wsUrl: wsUrl,
      nsec: nsec,
      onMessage: (_) {},
      onConnected: () {},
      onDisconnected: (_) {},
    );
    try {
      await socket.connect().timeout(const Duration(seconds: 8));
    } finally {
      await socket.disconnect();
    }
  }

  Community _parseLegacyInput(String raw) {
    var payload = raw.trim();

    if (payload.startsWith('buzz://')) {
      payload = payload.substring('buzz://'.length);
    }

    final normalized = base64Url.normalize(payload);
    final jsonStr = utf8.decode(base64Url.decode(normalized));
    final decoded = jsonDecode(jsonStr);
    if (decoded is! Map<String, dynamic>) {
      throw const FormatException('Pairing payload is not a JSON object');
    }

    final relayUrl = decoded['relayUrl'] as String?;
    if (relayUrl == null) {
      throw const FormatException('Missing relayUrl in payload');
    }

    _validateRelayUrl(relayUrl);

    return Community.create(
      name: Community.nameFromUrl(relayUrl),
      relayUrl: relayUrl,
      pubkey: decoded['pubkey'] as String?,
      nsec: decoded['nsec'] as String?,
      sensitiveActionPolicy: SensitiveActionPolicy.disabledByUser,
    );
  }

  void _validateRelayUrl(String url) {
    final uri = Uri.parse(url);

    if (!kDebugMode && uri.scheme != 'https') {
      throw const FormatException('Relay URL must use HTTPS');
    }
    if (uri.scheme != 'http' && uri.scheme != 'https') {
      throw FormatException('Invalid URL scheme: ${uri.scheme}');
    }

    final host = uri.host.toLowerCase();
    if (host == 'localhost' || host == '127.0.0.1' || host == '::1') {
      if (!kDebugMode) {
        throw const FormatException('Relay URL cannot target localhost');
      }
      return;
    }

    final ip = Uri.tryParse('http://$host')?.host ?? host;
    if (_isPrivateHost(ip)) {
      throw const FormatException(
        'Relay URL cannot target private network addresses',
      );
    }
  }

  static bool _isPrivateHost(String host) {
    final parts = host.split('.');
    if (parts.length != 4) return false;
    final octets = parts.map(int.tryParse).toList();
    if (octets.any((o) => o == null)) return false;

    final a = octets[0]!;
    final b = octets[1]!;

    if (a == 10) return true;
    if (a == 172 && b >= 16 && b <= 31) return true;
    if (a == 192 && b == 168) return true;
    if (a == 169 && b == 254) return true;
    return false;
  }
}

final pairingProvider = NotifierProvider<PairingNotifier, PairingState>(
  PairingNotifier.new,
);
