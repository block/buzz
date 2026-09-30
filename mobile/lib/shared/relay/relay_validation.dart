import 'dart:io';

import 'package:flutter/foundation.dart';

/// Validates an untrusted relay origin before the mobile app connects to it.
///
/// Production relay URLs must use TLS and may not contain local or otherwise
/// non-public IP literals. Debug builds retain plaintext localhost support for
/// local development, but other non-public destinations remain blocked.
void validateInviteRelayUri(
  Uri uri, {
  bool allowInsecureLocalhost = kDebugMode,
}) {
  if (uri.host.isEmpty ||
      uri.userInfo.isNotEmpty ||
      uri.hasFragment ||
      (uri.path.isNotEmpty && uri.path != '/') ||
      uri.hasQuery) {
    throw const FormatException('Relay URL must be an origin');
  }

  if (uri.scheme != 'ws' && uri.scheme != 'wss') {
    throw FormatException('Invalid relay URL scheme: ${uri.scheme}');
  }

  final host = uri.host.toLowerCase();
  final isLocalhost = host == 'localhost' || host.endsWith('.localhost');
  if (uri.scheme != 'wss' && !(allowInsecureLocalhost && isLocalhost)) {
    throw const FormatException('Relay URL must use WSS');
  }
  if (isLocalhost) {
    if (!allowInsecureLocalhost) {
      throw const FormatException('Relay URL cannot target localhost');
    }
    return;
  }

  final address = InternetAddress.tryParse(host);
  if (_looksLikeNumericAddress(host)) {
    // Only canonical dotted-decimal IPv4 is accepted. Ambiguous legacy forms
    // such as a single integer, hexadecimal, shortened components, or leading
    // zeroes have varied interpretation across URI/network stacks.
    final ipv4Parts = host.split('.');
    final hasLeadingZero = ipv4Parts.any(
      (part) => part.length > 1 && part.startsWith('0'),
    );
    if (address == null ||
        address.type != InternetAddressType.IPv4 ||
        address.address != host ||
        ipv4Parts.length != 4 ||
        hasLeadingZero) {
      throw const FormatException('Relay URL contains an invalid IP address');
    }
  }
  if (address != null) {
    if (!_isPublicAddress(address)) {
      throw const FormatException(
        'Relay URL cannot target non-public network addresses',
      );
    }
    return;
  }

  // DNS hostnames are allowed here. Preventing a hostname from resolving or
  // rebinding to a non-public address requires connect-time resolution and
  // address pinning in the networking layer.
}

bool _looksLikeNumericAddress(String host) {
  if (RegExp(r'^\d+$').hasMatch(host) ||
      RegExp(r'^0x[0-9a-f]+$', caseSensitive: false).hasMatch(host)) {
    return true;
  }
  final parts = host.split('.');
  return parts.length > 1 &&
      parts.every(
        (part) =>
            RegExp(r'^\d+$').hasMatch(part) ||
            RegExp(r'^0x[0-9a-f]+$', caseSensitive: false).hasMatch(part),
      );
}

bool _isPublicAddress(InternetAddress address) {
  final bytes = address.rawAddress;
  if (address.type == InternetAddressType.IPv4) {
    return !_isNonPublicIpv4(bytes);
  }
  if (address.type != InternetAddressType.IPv6 || bytes.length != 16) {
    return false;
  }

  // IPv4-compatible and IPv4-mapped IPv6 addresses inherit the embedded
  // IPv4 address's classification.
  final firstTenZero = bytes.take(10).every((byte) => byte == 0);
  if (firstTenZero &&
      ((bytes[10] == 0 && bytes[11] == 0) ||
          (bytes[10] == 0xff && bytes[11] == 0xff))) {
    return !_isNonPublicIpv4(bytes.sublist(12));
  }

  // Accept only globally reachable IPv6 literals. Global unicast space is
  // 2000::/3, with special-purpose exclusions and narrow globally reachable
  // exceptions tracked by IANA (reconciled 2026-07-26):
  // https://www.iana.org/assignments/iana-ipv6-special-registry/
  if (!_hasPrefix(bytes, [0x20], 3)) return false;

  // 2001::/23 is reserved for IETF protocol assignments. Only these entries
  // are currently classified by IANA as globally reachable.
  if (_hasPrefix(bytes, [0x20, 0x01, 0x00], 23) &&
      !_isGloballyReachableIetfAssignment(bytes)) {
    return false;
  }

  // Documentation and transition prefixes are not public relay destinations.
  if (_hasPrefix(bytes, [0x20, 0x01, 0x0d, 0xb8], 32)) return false;
  if (_hasPrefix(bytes, [0x20, 0x02], 16)) return false;
  if (_hasPrefix(bytes, [0x3f, 0xff, 0x00], 20)) return false;
  return true;
}

bool _isGloballyReachableIetfAssignment(List<int> bytes) {
  final isGlobalAnycast =
      _hasPrefix(bytes, [0x20, 0x01, 0x00, 0x01, 0, 0, 0, 0], 64) &&
      bytes.sublist(8, 15).every((byte) => byte == 0) &&
      bytes[15] >= 1 &&
      bytes[15] <= 3;
  return isGlobalAnycast ||
      _hasPrefix(bytes, [0x20, 0x01, 0x00, 0x03], 32) ||
      _hasPrefix(bytes, [0x20, 0x01, 0x00, 0x04, 0x01, 0x12], 48) ||
      _hasPrefix(bytes, [0x20, 0x01, 0x00, 0x20], 28) ||
      _hasPrefix(bytes, [0x20, 0x01, 0x00, 0x30], 28);
}

bool _hasPrefix(List<int> address, List<int> prefix, int prefixLength) {
  final wholeBytes = prefixLength ~/ 8;
  for (var i = 0; i < wholeBytes; i++) {
    if (address[i] != prefix[i]) return false;
  }
  final remainingBits = prefixLength % 8;
  if (remainingBits == 0) return true;
  final mask = (0xff << (8 - remainingBits)) & 0xff;
  return (address[wholeBytes] & mask) == (prefix[wholeBytes] & mask);
}

bool _isNonPublicIpv4(List<int> bytes) {
  if (bytes.length != 4) return true;
  final a = bytes[0];
  final b = bytes[1];
  final c = bytes[2];

  return a == 0 ||
      a == 10 ||
      a == 127 ||
      (a == 100 && b >= 64 && b <= 127) ||
      (a == 169 && b == 254) ||
      (a == 172 && b >= 16 && b <= 31) ||
      (a == 192 && b == 0 && c == 0) ||
      (a == 192 && b == 0 && c == 2) ||
      (a == 192 && b == 168) ||
      (a == 198 && (b == 18 || b == 19)) ||
      (a == 198 && b == 51 && c == 100) ||
      (a == 203 && b == 0 && c == 113) ||
      a >= 224;
}
