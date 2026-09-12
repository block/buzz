import 'package:nostr/nostr.dart' as nostr;

/// A completed Nostr event payload. No clock or tag construction runs in sign().
final class UnsignedEvent {
  final String publicKey;
  final int createdAt;
  final int kind;
  final String content;
  final List<List<String>> tags;

  UnsignedEvent({
    required this.publicKey,
    required this.createdAt,
    required this.kind,
    required this.content,
    required List<List<String>> tags,
  }) : tags = List.unmodifiable(
         tags.map((tag) => List<String>.unmodifiable(tag)),
       );
}

/// Signs an exact event, without login, key export, encryption or publication.
abstract interface class EventSigner {
  /// Public key bound to this operation's signer snapshot.
  String get publicKey;

  /// Sign the supplied fields without rewriting them or selecting another key.
  Future<nostr.Event> sign(UnsignedEvent event);
}

/// Local signing only. Secret storage, export and pairing remain separate APIs.
final class LocalEventSigner implements EventSigner {
  final String _secretKey;

  /// Capture an explicitly supplied hex secret; never generate or look up keys.
  LocalEventSigner(String secretKey) : _secretKey = secretKey;

  @override
  String get publicKey => nostr.Keys(_secretKey).public;

  @override
  Future<nostr.Event> sign(UnsignedEvent event) async {
    if (event.publicKey != publicKey) {
      throw StateError('Unsigned event does not match the signing identity');
    }
    return nostr.Event.from(
      kind: event.kind,
      content: event.content,
      tags: event.tags,
      createdAt: event.createdAt,
      pubkey: event.publicKey,
      secretKey: _secretKey,
      verify: false,
    );
  }
}

/// Construct an event before passing it to the signer. Transport stays with callers.
Future<nostr.Event> signEvent({
  required EventSigner signer,
  required int kind,
  required String content,
  required List<List<String>> tags,
  int? createdAt,
}) {
  final event = UnsignedEvent(
    publicKey: signer.publicKey,
    createdAt: createdAt ?? DateTime.now().millisecondsSinceEpoch ~/ 1000,
    kind: kind,
    content: content,
    tags: tags,
  );
  return signer.sign(event);
}
