/// Onion-authenticated WebSocket transport (no CA required).
///
/// A v3 `.onion` address *is* the hidden service's ed25519 public key, and
/// Tor's rendezvous handshake cryptographically proves the server holds the
/// matching private key **before** any app traffic flows. That means the
/// server is already authenticated at the transport layer — exactly the job a
/// TLS certificate normally does. Over `.onion`, TLS therefore only provides
/// (redundant) encryption, so the certificate does not need to chain to a CA.
///
/// This helper builds an [HttpClient] that accepts an otherwise-untrusted TLS
/// certificate **iff** the host is an onion address, and wraps it into a
/// [WebSocketChannel]. For every non-onion (clearnet) host the default
/// web-PKI verification is left completely untouched.
library;

import 'dart:io';

import 'package:web_socket_channel/io.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

/// Returns true iff [host] is a Tor onion address.
///
/// Strict exact-suffix match on a parsed host (never a substring test):
/// the host must end with the `.onion` label and have a non-empty label in
/// front of it. `evil.onion.example.com` does NOT match (it does not end in
/// `.onion`); `notonion` does not match (no dot). Comparison is
/// case-insensitive.
bool isOnionHost(String host) {
  final h = host.toLowerCase();
  const suffix = '.onion';
  if (!h.endsWith(suffix)) return false;
  // Require a non-empty label before ".onion" (reject a bare ".onion").
  return h.length > suffix.length;
}

/// An [HttpClient] that trusts an otherwise-invalid TLS certificate **only**
/// when the connection target is an onion host.
///
/// The [HttpClient.badCertificateCallback] is invoked by dart:io only when the
/// default trust evaluation has already failed. Returning `true` overrides that
/// failure; returning `false` preserves the default (web-PKI) rejection. We
/// override **iff** [isOnionHost] — safe because `.onion` resolution only
/// happens over Tor, which has already authenticated the server by its key.
///
/// Defense-in-depth (verifier option B) could additionally require the cert's
/// SAN to include [host]; option A (onion-transport trust) is intentionally
/// minimal and is what ships here.
HttpClient onionAwareHttpClient() {
  final client = HttpClient();
  client.badCertificateCallback =
      (X509Certificate cert, String host, int port) {
        return isOnionHost(host);
      };
  return client;
}

/// Connects a [WebSocketChannel] using the onion-aware TLS trust policy.
///
/// Synchronous, matching `WebSocketChannel.connect`'s signature so it can be
/// dropped in as a channel factory. The returned channel connects lazily and
/// exposes readiness via [WebSocketChannel.ready]. Only `wss://<onion>` traffic
/// is affected by the relaxed trust; `wss://` to any clearnet host still goes
/// through full web-PKI verification, and `ws://` (plain) is unchanged.
WebSocketChannel onionAwareChannel(Uri uri) {
  return IOWebSocketChannel.connect(
    _withExplicitPort(uri),
    customClient: onionAwareHttpClient(),
  );
}

/// Make the port explicit so it is never resolved to 0.
///
/// dart:io's `WebSocket.connect` builds the HTTP upgrade request straight from
/// [Uri.port], but Dart's [Uri] only defines default ports for the `http`
/// (80) and `https` (443) schemes — for `ws`/`wss` with no explicit port,
/// [Uri.port] returns **0**. That leaks a `:0` into the connect target (e.g.
/// `http://<host>:0`), which the transport (Tor/Orbot here) immediately resets.
/// A `ws://<onion>` payload with no port therefore fails until the default is
/// filled in here. Only touches URIs that omit the port; explicit ports (and
/// non-ws schemes) pass through untouched.
Uri _withExplicitPort(Uri uri) {
  if (uri.hasPort) return uri;
  switch (uri.scheme) {
    case 'wss':
    case 'https':
      return uri.replace(port: 443);
    case 'ws':
    case 'http':
      return uri.replace(port: 80);
    default:
      return uri;
  }
}
