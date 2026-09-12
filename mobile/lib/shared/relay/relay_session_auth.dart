part of 'relay_session.dart';

Future<String> buildNip98AuthHeader({
  required String method,
  required String url,
  required List<int> bodyBytes,
  required String? nsec,
  EventSigner? signer,
}) async {
  signer ??= nsec == null
      ? null
      : LocalEventSigner(nostr.Nip19.decode(payload: nsec).data);
  if (signer == null) throw StateError('No signing identity');
  final payloadHash = SHA256Digest()
      .process(Uint8List.fromList(bodyBytes))
      .map((byte) => byte.toRadixString(16).padLeft(2, '0'))
      .join();
  final event = await signEvent(
    kind: 27235,
    content: '',
    tags: [
      ['u', url],
      ['method', method.toUpperCase()],
      ['payload', payloadHash],
      ['nonce', const Uuid().v4()],
    ],
    signer: signer,
  );
  return 'Nostr ${base64.encode(utf8.encode(event.toJson()))}';
}
