import 'dart:typed_data';

import '../../shared/crypto/ecdh.dart';
import '../../shared/crypto/hkdf.dart';

/// Derive session ID from session secret.
/// session_id = HKDF-SHA256(IKM=session_secret, salt="", info="nostr-pair-session-id", L=32)
Uint8List deriveSessionId(Uint8List sessionSecret) {
  return hkdf(
    sessionSecret,
    Uint8List(0), // empty salt
    Uint8List.fromList('nostr-pair-session-id'.codeUnits),
    32,
  );
}

/// Derive SAS code and SAS input from ECDH shared secret and session secret.
/// sas_input = HKDF-SHA256(IKM=ecdh_shared, salt=session_secret, info="nostr-pair-sas-v1", L=32)
/// sas_code = be_u32(sas_input[0..4]) % 1_000_000
(int, Uint8List) deriveSas(Uint8List ecdhShared, Uint8List sessionSecret) {
  final sasInput = hkdf(
    ecdhShared,
    sessionSecret,
    Uint8List.fromList('nostr-pair-sas-v1'.codeUnits),
    32,
  );

  // be_u32 from first 4 bytes.
  final code =
      ((sasInput[0] << 24) |
          (sasInput[1] << 16) |
          (sasInput[2] << 8) |
          sasInput[3]) %
      1000000;

  return (code, sasInput);
}

/// Format a SAS code as a 6-digit zero-padded string.
String formatSas(int code) => code.toString().padLeft(6, '0');

/// Derive transcript hash binding all session parameters.
/// transcript = session_id || source_pubkey || target_pubkey || sas_input (128 bytes)
/// transcript_hash = HKDF-SHA256(IKM=transcript, salt=session_secret, info="nostr-pair-transcript-v1", L=32)
Uint8List deriveTranscriptHash(
  Uint8List sessionId,
  Uint8List sourcePubkey,
  Uint8List targetPubkey,
  Uint8List sasInput,
  Uint8List sessionSecret,
) {
  final transcript = Uint8List(128);
  transcript.setRange(0, 32, sessionId);
  transcript.setRange(32, 64, sourcePubkey);
  transcript.setRange(64, 96, targetPubkey);
  transcript.setRange(96, 128, sasInput);

  return hkdf(
    transcript,
    sessionSecret,
    Uint8List.fromList('nostr-pair-transcript-v1'.codeUnits),
    32,
  );
}

/// Parse a `nostrpair://` QR URI.
({
  String sourcePubkey,
  Uint8List sessionSecret,
  List<String> relays,
  int version,
})
parseNostrpairUri(String uri) {
  if (uri.length > 2048) {
    throw FormatException('URI exceeds 2048-character limit');
  }

  if (!uri.startsWith('nostrpair://')) {
    throw const FormatException('URI must start with nostrpair://');
  }

  final rest = uri.substring('nostrpair://'.length);
  final qIdx = rest.indexOf('?');
  if (qIdx < 0) {
    throw const FormatException('Missing query string');
  }

  final pubkeyHex = rest.substring(0, qIdx);
  final query = rest.substring(qIdx + 1);

  // Validate pubkey: 64 lowercase hex chars.
  if (pubkeyHex.length != 64 || !_isLowercaseHex(pubkeyHex)) {
    throw const FormatException('Pubkey must be 64 lowercase hex chars');
  }

  // Parse query params.
  String? secretHex;
  final relays = <String>[];
  int? version;

  for (final pair in query.split('&')) {
    final eqIdx = pair.indexOf('=');
    if (eqIdx < 0) continue;
    final key = pair.substring(0, eqIdx);
    final value = pair.substring(eqIdx + 1);
    switch (key) {
      case 'secret':
        secretHex = value;
      case 'relay':
        relays.add(Uri.decodeComponent(value));
      case 'v':
        version = int.tryParse(value);
    }
  }

  // Default version to 1.
  version ??= 1;
  if (version != 1) {
    throw FormatException(
      'Unsupported protocol version $version. Please update the app.',
    );
  }

  // Validate secret.
  if (secretHex == null ||
      secretHex.length != 64 ||
      !_isLowercaseHex(secretHex)) {
    throw const FormatException('Secret must be 64 lowercase hex chars');
  }
  final sessionSecret = hexToBytes(secretHex);

  // Reject all-zeros.
  if (sessionSecret.every((b) => b == 0)) {
    throw const FormatException('Session secret must not be all zeros');
  }

  if (relays.isEmpty) {
    throw const FormatException('At least one relay URL is required');
  }

  // Validate relay URLs.
  for (final relay in relays) {
    final parsed = Uri.tryParse(relay);
    if (parsed == null || (parsed.scheme != 'wss' && parsed.scheme != 'ws')) {
      throw FormatException('Invalid relay URL: $relay');
    }
  }

  return (
    sourcePubkey: pubkeyHex,
    sessionSecret: sessionSecret,
    relays: relays,
    version: version,
  );
}

bool _isLowercaseHex(String s) {
  for (final c in s.codeUnits) {
    if (!((c >= 0x30 && c <= 0x39) || (c >= 0x61 && c <= 0x66))) {
      return false;
    }
  }
  return true;
}
