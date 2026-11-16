part of 'relay_session.dart';

/// The auth fields the session depends on: sign-in status and identity.
/// Watching only these keeps an auth reload, such as an invalidation that
/// re-reads the same community, from dropping the socket and subscriptions.
({AuthStatus? status, String? pubkey}) _sessionAuthKey(
  AsyncValue<AuthState> auth,
) => (status: auth.value?.status, pubkey: auth.value?.community?.pubkey);

/// Pass the signer's hex [pubkey] when it is already known: deriving it from
/// the key costs about a third of a signature, on the UI isolate.
String buildNip98AuthHeader({
  required String method,
  required String url,
  required List<int> bodyBytes,
  required String? nsec,
  String? pubkey,
}) {
  if (nsec == null || nsec.isEmpty) {
    throw Exception('Cannot query relay: no signing key available');
  }
  final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
  if (privkeyHex.isEmpty) {
    throw Exception('Invalid nsec');
  }
  final payloadHash = crypto.sha256.convert(bodyBytes).toString();
  final event = signNostrEvent(
    kind: 27235,
    content: '',
    tags: [
      ['u', url],
      ['method', method.toUpperCase()],
      ['payload', payloadHash],
      ['nonce', const Uuid().v4()],
    ],
    secretKey: privkeyHex,
    pubkey: pubkey,
  );
  return 'Nostr ${base64.encode(utf8.encode(event.toJson()))}';
}
