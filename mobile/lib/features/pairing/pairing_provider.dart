import 'dart:async';
import 'dart:convert';
import 'dart:math' as math;

import 'package:flutter/foundation.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;

import '../../shared/auth/auth.dart';
import '../../shared/community/paired_community_landing.dart';
import '../../shared/crypto/ecdh.dart';
import '../../shared/crypto/nip44.dart';
import '../../shared/relay/relay.dart';
import '../../shared/security/sensitive_action_authorizer.dart';
import 'pairing_crypto.dart';
import 'pairing_socket.dart';

part 'pairing_provider_helpers.dart';
part 'pairing_state.dart';

/// HTTP client used by [PairingNotifier] for the validation request.
final pairingHttpClientProvider = Provider<http.Client>((ref) {
  final client = http.Client();
  ref.onDispose(client.close);
  return client;
});

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
  final RelaySocketFactory _validationSocketFactory;
  RelaySocket? _validationSocket;
  PairingSocket? _socket;
  Timer? _sessionTimeout;
  Completer<bool>? _codeResult;
  String? _codeRequestId;
  int _codeRequestSequence = 0;
  Community? _identityExportCommunity;
  bool _identityExportBiometricOnly = false;

  PairingNotifier({
    PairingSocketFactory? socketFactory,
    PairingCredentialValidator? credentialValidator,
    RelaySocketFactory validationSocketFactory = RelaySocket.new,
  }) : _socketFactory = socketFactory ?? _createPairingSocket,
       _credentialValidator = credentialValidator,
       _validationSocketFactory = validationSocketFactory;

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
  PairingState build() {
    ref.onDispose(_cleanup);
    return const PairingState();
  }

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

  /// Check user input with the source; never compare a QR-derived SAS when the
  /// source has negotiated an independent desktop-only code.
  Future<bool> verifyDesktopCode(String code) async {
    if (!state.requiresDesktopCode ||
        state.status != PairingStatus.confirmingSas ||
        _codeResult != null ||
        !RegExp(r'^\d{6}$').hasMatch(code)) {
      return false;
    }
    final result = Completer<bool>();
    _codeResult = result;
    _codeRequestId = '${++_codeRequestSequence}';
    _publishEvent(
      kind: 24134,
      content: _encryptMessage({
        'type': 'code-submit',
        'code': code,
        'request_id': _codeRequestId,
      }),
      tags: [
        ['p', _sourcePubkey!],
      ],
    );
    try {
      // Keep this logical attempt pending until its response or session cleanup.
      // A local deadline cannot undo a guess already consumed by the source.
      return await result.future;
    } finally {
      if (identical(_codeResult, result)) {
        _codeResult = null;
        _codeRequestId = null;
      }
    }
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
    if (_codeResult?.isCompleted == false) _codeResult!.complete(false);
    _codeResult = null;
    _codeRequestId = null;
    _pairingGeneration++;
    _validationSocket?.dispose();
    _validationSocket = null;
    _sessionTimeout?.cancel();
    _sessionTimeout = null;
    _socket?.dispose();
    _socket = null;
    _processedEventIds.clear();
    _publishedEventIds.clear();
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
  final Set<String> _publishedEventIds = {};
  final Set<String> _processedEventIds = {}; // NIP-AB §Duplicate Event Handling

  Future<void> _pairNipAb(String uri) async {
    final generation = _pairingGeneration;
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
        onMessage: (message) {
          if (generation == _pairingGeneration) _handleRelayMessage(message);
        },
        onDisconnected: (error) {
          if (generation == _pairingGeneration) _handleDisconnected(error);
        },
      );
      _socket = socket;
      await socket.connect();
      if (generation != _pairingGeneration) return;

      if (!socket.isConnected) {
        throw StateError('Pairing socket did not reach the connected state');
      }

      // 5. Subscribe for kind:24134 events tagged to our ephemeral pubkey.
      socket.subscribe('pair', 24134, _ephemeralPubkey!);

      // 6. Wait briefly for EOSE, then send offer.
      // (In practice, we send the offer immediately — the relay will buffer it.)
      await Future.delayed(const Duration(milliseconds: 500));
      if (generation != _pairingGeneration) return;

      // 7. Build and send the offer event.
      final offerContent = _encryptMessage({
        'type': 'offer',
        if (!_sendIdentityToSource) 'confirmation': 'desktop-code-v1',
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
      if (generation != _pairingGeneration) return;
      _cleanup();
      state = PairingState(
        status: PairingStatus.error,
        errorMessage: 'Invalid pairing code: ${e.message}',
      );
    } catch (e) {
      if (generation != _pairingGeneration) return;
      debugPrint('Pairing connection error: $e');
      _cleanup();
      state = PairingState(
        status: PairingStatus.error,
        errorMessage: _friendlyErrorMessage(e),
      );
    }
  }

  void _handleRelayMessage(List<dynamic> data) {
    if (data.isEmpty) return;
    final type = data[0] as String;

    final rejected =
        type == 'OK' &&
        data.length >= 3 &&
        _publishedEventIds.remove(data[1]) &&
        data[2] == false;
    final closed = type == 'CLOSED' && data.length >= 2 && data[1] == 'pair';
    if ((rejected || closed) &&
        state.status != PairingStatus.success &&
        state.status != PairingStatus.error) {
      _cleanup();
      state = const PairingState(
        status: PairingStatus.error,
        errorMessage:
            'Pairing couldn’t continue. Scan a new desktop QR code and try again.',
      );
      return;
    }
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
        case 'desktop-code':
          if (!_sendIdentityToSource &&
              state.status == PairingStatus.confirmingSas &&
              !_sasConfirmReceived) {
            state = state.copyWith(requiresDesktopCode: true);
          }
          _processedEventIds.add(eventId);
        case 'code-rejected':
          if (state.requiresDesktopCode &&
              msg['request_id'] == _codeRequestId &&
              _codeResult?.isCompleted == false) {
            _codeResult!.complete(false);
            if (msg['remaining_attempts'] == 0) {
              _cleanup();
              state = const PairingState(
                status: PairingStatus.error,
                errorMessage:
                    'Too many incorrect codes. Scan a new desktop QR code and try again.',
              );
            }
          }
          _processedEventIds.add(eventId);
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
    if (state.requiresDesktopCode && _codeResult?.isCompleted == false) {
      _codeResult!.complete(true);
    }

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
      state = state.copyWith(destinationRelayUrl: relayUrl);

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
      state = state.copyWith(status: PairingStatus.success);
      ref.read(pairedCommunityLandingProvider.notifier).request(community);
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

    _publishedEventIds.add(event.id);
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
    final generation = _pairingGeneration;
    state = const PairingState(status: PairingStatus.connecting);

    try {
      final community = _parseLegacyInput(rawInput);
      final validator = _credentialValidator ?? _validateCredentials;
      await validator(relayUrl: community.relayUrl, nsec: community.nsec);

      if (generation != _pairingGeneration) return;
      await ref
          .read(authProvider.notifier)
          .authenticateWithCommunity(community);
      if (generation != _pairingGeneration) return;
      state = PairingState(
        status: PairingStatus.success,
        destinationRelayUrl: community.relayUrl,
      );
      ref.read(pairedCommunityLandingProvider.notifier).request(community);
    } on FormatException catch (e) {
      if (generation != _pairingGeneration) return;
      state = PairingState(
        status: PairingStatus.error,
        errorMessage: 'Invalid pairing code: ${e.message}',
      );
    } on RelayException catch (e) {
      if (generation != _pairingGeneration) return;
      state = PairingState(
        status: PairingStatus.error,
        errorMessage:
            'Could not connect to relay (${e.statusCode}). '
            'Check that the pairing code is valid.',
      );
    } catch (e) {
      if (generation != _pairingGeneration) return;
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

    // connect() reports handshake and auth failures via onDisconnected and
    // then returns. Pairing must not store credentials when that happens (#8043).
    Object? failure;
    final socket = _validationSocketFactory(
      wsUrl: wsUrl,
      nsec: nsec,
      onMessage: (_) {},
      onConnected: () {},
      onDisconnected: (error) {
        failure ??= error ?? Exception('Could not connect to relay');
      },
    );
    _validationSocket = socket;
    try {
      await socket.connect().timeout(const Duration(seconds: 8));
      if (failure != null) {
        throw failure!;
      }
      if (socket.state != SocketState.connected) {
        throw Exception('Could not connect to relay');
      }
    } finally {
      if (identical(_validationSocket, socket)) _validationSocket = null;
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
}

final pairingProvider = NotifierProvider<PairingNotifier, PairingState>(
  PairingNotifier.new,
);
