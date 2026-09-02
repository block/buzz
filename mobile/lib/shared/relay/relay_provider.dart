import 'dart:io';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../community/community_provider.dart';
import 'relay_client.dart';

const _relayConfigSentinel = Object();

bool _isPrivateIpv4(String hostname) {
  final octets = hostname.split('.').map(int.tryParse).toList();
  if (octets.length != 4 || octets.any((octet) => octet == null)) {
    return false;
  }
  final values = octets.cast<int>();
  if (values.any((octet) => octet < 0 || octet > 255)) return false;
  final first = values[0];
  final second = values[1];
  return first == 10 ||
      first == 127 ||
      (first == 169 && second == 254) ||
      (first == 172 && second >= 16 && second <= 31) ||
      (first == 192 && second == 168) ||
      (first == 100 && second >= 64 && second <= 127);
}

bool _isPrivateIpv6(String hostname) {
  final address = InternetAddress.tryParse(hostname);
  if (address == null || address.type != InternetAddressType.IPv6) return false;
  final bytes = address.rawAddress;
  if (bytes.length != 16) return false;
  final isLoopback =
      bytes.take(15).every((byte) => byte == 0) && bytes[15] == 1;
  final isUniqueLocal = (bytes[0] & 0xfe) == 0xfc;
  final isLinkLocal = bytes[0] == 0xfe && (bytes[1] & 0xc0) == 0x80;
  return isLoopback || isUniqueLocal || isLinkLocal;
}

bool _isPrivateLanHost(String hostname) {
  final normalized = hostname.toLowerCase();
  return normalized == 'localhost' ||
      _isPrivateIpv4(normalized) ||
      _isPrivateIpv6(normalized);
}

/// Normalize and validate an optional plaintext private-network relay.
String? normalizeLanRelayUrl(String input) {
  final trimmed = input.trim();
  if (trimmed.isEmpty) return null;

  final withScheme = trimmed.contains('://') ? trimmed : 'ws://$trimmed';
  final uri = Uri.tryParse(withScheme);
  if (uri == null || uri.host.isEmpty) {
    throw const FormatException(
      'Enter a valid ws:// private-network relay URL.',
    );
  }
  if (uri.scheme != 'ws') {
    throw const FormatException('The Campus / LAN relay must use ws://.');
  }
  if (!_isPrivateLanHost(uri.host)) {
    throw const FormatException(
      'The Campus / LAN relay must use localhost or a private IP address.',
    );
  }
  if (uri.userInfo.isNotEmpty || uri.hasQuery || uri.hasFragment) {
    throw const FormatException(
      'The Campus / LAN relay URL cannot contain credentials or parameters.',
    );
  }
  if (uri.path.isNotEmpty && uri.path != '/') {
    throw const FormatException(
      'The Campus / LAN relay URL must not contain a path.',
    );
  }
  return Uri(
    scheme: 'ws',
    host: uri.host,
    port: uri.hasPort ? uri.port : null,
  ).toString();
}

/// Relay connection configuration.
///
/// In the pure-nostr world the only secrets the app cares about are:
///   - `baseUrl` — where the relay lives (used for WS + media upload)
///   - `nsec`    — the user's signing key (drives NIP-42 AUTH and event sigs)
class RelayConfig {
  const RelayConfig({required String baseUrl, this.lanRelayUrl, this.nsec})
    : _baseUrl = baseUrl;

  /// Relay origin exactly as the active community stored it.
  final String _baseUrl;

  /// Optional private-network WebSocket transport tried before [wsUrl].
  final String? lanRelayUrl;

  /// Nostr secret key (bech32 nsec) for signing events and NIP-42 AUTH.
  final String? nsec;

  /// The origin as persisted, before scheme canonicalization.
  ///
  /// Exists solely so identity-scoped storage keys written before [baseUrl]
  /// was canonicalized stay reachable — see [readMigratedPref]. Never use it
  /// for network I/O; [baseUrl] and [wsUrl] are the addresses to connect to.
  String get storedOrigin => _baseUrl;

  /// Relay origin as an HTTP(S) URL.
  ///
  /// Communities are persisted with whichever scheme their onboarding flow
  /// used: device pairing stores `https://` (it rejects anything else), while
  /// an invite join stores the `wss://` relay URL carried by the invite link.
  /// Every consumer treats this as an HTTP origin — [wsUrl], the `/query`
  /// endpoint, media upload and Blossom auth — so a `wss://` base silently
  /// degrades all of them. Folding the websocket schemes back here keeps both
  /// onboarding paths equivalent, including for already-persisted communities.
  ///
  /// Derived rather than normalized in the constructor so that the constructor
  /// stays `const`: the compile-time fallback below relies on canonicalization
  /// to keep its identity stable across rebuilds, and Riverpod's default
  /// `updateShouldNotify` is `previous != next`, which falls back to identity
  /// here. A fresh instance per rebuild would resubscribe every listener.
  String get baseUrl {
    final uri = Uri.tryParse(_baseUrl);
    if (uri == null) return _baseUrl;
    final scheme = switch (uri.scheme) {
      'wss' => 'https',
      'ws' => 'http',
      _ => null,
    };
    return scheme == null ? _baseUrl : uri.replace(scheme: scheme).toString();
  }

  /// Derive the websocket URL from the HTTP base URL.
  String get wsUrl {
    final uri = Uri.parse(baseUrl);
    final scheme = uri.scheme == 'https' ? 'wss' : 'ws';
    return uri.replace(scheme: scheme).toString();
  }

  /// WebSocket transports in connection priority order.
  List<String> get wsUrls => [
    if (lanRelayUrl != null && lanRelayUrl != wsUrl) lanRelayUrl!,
    wsUrl,
  ];

  /// HTTP relay origins in request priority order.
  List<String> get httpBaseUrls {
    final lanHttpUrl = lanRelayUrl == null
        ? null
        : Uri.parse(lanRelayUrl!).replace(scheme: 'http').toString();
    return [
      if (lanHttpUrl != null && lanHttpUrl != baseUrl) lanHttpUrl,
      baseUrl,
    ];
  }
}

/// Compile-time environment config via --dart-define.
///
/// Run with:
///   flutter run --dart-define=BUZZ_RELAY_URL=http://localhost:3000
///
/// Or create a `.env.json` and use --dart-define-from-file=.env.json
class Env {
  static const relayUrl = String.fromEnvironment(
    'BUZZ_RELAY_URL',
    defaultValue: 'http://localhost:3000',
  );

  static const lanRelayUrl = String.fromEnvironment('BUZZ_LAN_RELAY_URL');
}

class RelayConfigNotifier extends Notifier<RelayConfig> {
  @override
  RelayConfig build() {
    // Watch the active community so that when it changes (community switch),
    // the config rebuilds, triggering the full provider cascade.
    final activeAsync = ref.watch(activeCommunityProvider);
    final active = activeAsync.value;
    if (active != null) {
      return RelayConfig(
        baseUrl: active.relayUrl,
        lanRelayUrl: active.lanRelayUrl,
        nsec: active.nsec,
      );
    }

    // Fallback to compile-time env config (dev mode).
    return const RelayConfig(
      baseUrl: Env.relayUrl,
      lanRelayUrl: Env.lanRelayUrl == '' ? null : Env.lanRelayUrl,
    );
  }

  void update({
    required String baseUrl,
    Object? lanRelayUrl = _relayConfigSentinel,
    String? nsec,
  }) {
    state = RelayConfig(
      baseUrl: baseUrl,
      lanRelayUrl: lanRelayUrl == _relayConfigSentinel
          ? state.lanRelayUrl
          : lanRelayUrl as String?,
      nsec: nsec,
    );
  }
}

final relayConfigProvider = NotifierProvider<RelayConfigNotifier, RelayConfig>(
  RelayConfigNotifier.new,
);

/// Derive the hex pubkey from a bech32 nsec, or null on any failure.
String? pubkeyFromNsec(String? nsec) {
  if (nsec == null || nsec.isEmpty) return null;
  try {
    final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
    if (privkeyHex.isEmpty) return null;
    return nostr.Keys(privkeyHex).public;
  } catch (_) {
    return null;
  }
}

/// The current user's hex pubkey, derived from the active community nsec.
final myPubkeyProvider = Provider<String?>((ref) {
  final config = ref.watch(relayConfigProvider);
  return pubkeyFromNsec(config.nsec);
});

/// Provides a [RelayClient] that reacts to config changes.
///
/// Only used for the media upload HTTP endpoint now — all data flow goes
/// through the relay WebSocket session.
final relayClientProvider = Provider<RelayClient>((ref) {
  final config = ref.watch(relayConfigProvider);
  final client = RelayClient(baseUrl: config.baseUrl);
  ref.onDispose(client.dispose);
  return client;
});
