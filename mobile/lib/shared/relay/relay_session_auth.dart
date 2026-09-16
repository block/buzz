part of 'relay_session.dart';

String buildNip98AuthHeader({
  required String method,
  required String url,
  required List<int> bodyBytes,
  required String? nsec,
}) {
  if (nsec == null || nsec.isEmpty) {
    throw Exception('Cannot query relay: no signing key available');
  }
  final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
  if (privkeyHex.isEmpty) {
    throw Exception('Invalid nsec');
  }
  final payloadHash = SHA256Digest()
      .process(Uint8List.fromList(bodyBytes))
      .map((byte) => byte.toRadixString(16).padLeft(2, '0'))
      .join();
  final event = nostr.Event.from(
    kind: 27235,
    content: '',
    tags: [
      ['u', url],
      ['method', method.toUpperCase()],
      ['payload', payloadHash],
      ['nonce', const Uuid().v4()],
    ],
    secretKey: privkeyHex,
    verify: false,
  );
  return 'Nostr ${base64.encode(utf8.encode(event.toJson()))}';
}

Future<String> buildClientNip98AuthHeader({
  required String method,
  required String url,
  required List<int> bodyBytes,
  required String? nsec,
}) async {
  if (!enterpriseEnabled) {
    return buildNip98AuthHeader(
      method: method,
      url: url,
      bodyBytes: bodyBytes,
      nsec: nsec,
    );
  }
  final hash = SHA256Digest()
      .process(Uint8List.fromList(bodyBytes))
      .map((b) => b.toRadixString(16).padLeft(2, '0'))
      .join();
  final event = await signClientEvent(
    nsec: null,
    kind: 27235,
    content: '',
    tags: [
      ['u', url],
      ['method', method.toUpperCase()],
      ['payload', hash],
      ['nonce', const Uuid().v4()],
    ],
  );
  return 'Nostr ${base64.encode(utf8.encode(event.toJson()))}';
}
