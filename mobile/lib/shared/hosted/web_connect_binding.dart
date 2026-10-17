import 'dart:convert';

import 'package:nostr/nostr.dart' as nostr;

import '../deeplink/deep_link.dart';
import 'hosted_environment.dart';

/// The owner supplies keys only inside this callback, after authenticating and
/// resolving identity. Neither this service nor its result retains a secret.
typedef WithWebConnectKeys =
    Future<T> Function<T>(Future<T> Function(nostr.Keys keys) operation);

typedef VerifyWebConnectBinding =
    Future<String> Function({
      required String subject,
      required String challengeId,
      required String nonce,
      required String signedPayload,
    });

/// Binds a web challenge using the authenticated account's verification API.
/// The API must consume it for [subject] and return the verified bound pubkey.
/// A signature is never returned to a browser: shape checks alone cannot prove
/// that an incoming challenge belongs to the account signed into the app.
class WebConnectBindingService {
  final DateTime Function() now;
  WebConnectBindingService({DateTime Function()? now})
    : now = now ?? DateTime.now;

  Future<Uri> complete({
    required NostrBindDeepLink request,
    required String subject,
    required String expectedPubkey,
    required WithWebConnectKeys withKeys,
    required VerifyWebConnectBinding verify,
    bool Function()? isCurrent,
  }) async {
    void checkCurrent() {
      if (isCurrent != null && !isCurrent()) {
        throw const FormatException('Binding attempt cancelled');
      }
    }

    checkCurrent();
    if (subject.isEmpty || !request.expiresAt.isAfter(now())) {
      throw const FormatException('Binding request expired or unauthenticated');
    }
    final signed = await withKeys<({String payload, String pubkey})>((
      keys,
    ) async {
      if (keys.public.toLowerCase() != expectedPubkey.toLowerCase()) {
        throw const FormatException('Identity does not match');
      }
      final event = nostr.Event.from(
        kind: 24243,
        content: '',
        tags: [
          for (final field in const [
            'challenge_id',
            'nonce',
            'verification_code',
            'audience',
            'action',
            'protocol',
            'version',
            'origin',
            'expires_at',
          ])
            [field, request.fields[field]!],
        ],
        secretKey: keys.secret,
        verify: false,
      );
      // Encode through the library's canonical event representation.
      final payload = event.toJson();
      if (jsonDecode(payload) is! Map) {
        throw const FormatException('Invalid signed event');
      }
      return (payload: payload, pubkey: keys.public);
    });
    checkCurrent();
    final boundPubkey = await verify(
      subject: subject,
      challengeId: request.challengeId,
      nonce: request.nonce,
      signedPayload: signed.payload,
    );
    if (boundPubkey.toLowerCase() != signed.pubkey.toLowerCase()) {
      throw const FormatException('Verified identity does not match');
    }
    return HostedEnvironment.origin.resolve('/buzz');
  }
}
