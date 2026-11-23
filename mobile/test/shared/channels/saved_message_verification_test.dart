import 'package:buzz/shared/channels/saved_message_verification.dart';
import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:flutter_test/flutter_test.dart';

const _relaySecret =
    '0000000000000000000000000000000000000000000000000000000000000001';
const _userSecret =
    '0000000000000000000000000000000000000000000000000000000000000002';
final _relay = derivePublicKeyHex(_relaySecret);
final _user = derivePublicKeyHex(_userSecret);

NostrEvent _signed(int kind, String secret, {String content = '{}'}) =>
    NostrEvent.fromJson(
      signNostrEvent(
        kind: kind,
        content: content,
        secretKey: secret,
        createdAt: 1700000000,
        tags: const [
          ['h', '00000000-0000-4000-8000-000000000001'],
          ['e', 'ab'],
        ],
      ).toMap(),
    );

NostrEvent _with(
  NostrEvent event, {
  String? id,
  String? pubkey,
  String? content,
  String? sig,
}) => NostrEvent(
  id: id ?? event.id,
  pubkey: pubkey ?? event.pubkey,
  createdAt: event.createdAt,
  kind: event.kind,
  tags: event.tags,
  content: content ?? event.content,
  sig: sig ?? event.sig,
);

String _flipLast(String hex) =>
    hex.substring(0, hex.length - 1) + (hex.endsWith('0') ? '1' : '0');

void main() {
  const relayKinds = [39000, 39002, 39005, 39006, 40099, 48101, 48102];
  const mixedKinds = [5, 7, 9, 9005, 40002, 40003, 45001, 45003, 48100, 48103];

  test('relay-authored kinds bind to the relay key', () {
    for (final kind in relayKinds) {
      final byRelay = _signed(kind, _relaySecret);
      final byUser = _signed(kind, _userSecret);
      final cases = <(NostrEvent, String?, bool, String)>[
        (byRelay, _relay, true, 'relay-signed'),
        (byUser, _relay, false, 'member-signed forgery'),
        (byRelay, null, false, 'relay key unknown'),
        (byRelay, _relay.toUpperCase(), false, 'uppercase relay key'),
        (byRelay, '', false, 'empty relay key'),
        (byUser, _user, true, 'binds to whatever key is configured'),
      ];
      for (final (event, author, expected, label) in cases) {
        expect(
          verifySavedMessageEvent(event, relayAuthor: author),
          expected,
          reason: 'kind $kind: $label',
        );
      }
    }
  });

  test('mixed-author kinds pass on their own proof', () {
    for (final kind in mixedKinds) {
      for (final (secret, author) in [
        (_userSecret, _relay),
        (_userSecret, null),
        (_relaySecret, _relay),
        (_relaySecret, null),
      ]) {
        expect(
          verifySavedMessageEvent(
            _signed(kind, secret, content: 'hello'),
            relayAuthor: author,
          ),
          isTrue,
          reason: 'kind $kind, relay key $author',
        );
      }
    }
  });

  test('broken proofs are refused for every kind', () {
    for (final kind in [...relayKinds, ...mixedKinds]) {
      final event = _signed(kind, _relaySecret);
      final other = _signed(kind, _relaySecret, content: 'other');
      final cases = <(NostrEvent, String)>[
        (_with(event, content: 'tampered'), 'content changed'),
        (_with(event, id: _flipLast(event.id)), 'id changed'),
        (_with(event, sig: _flipLast(event.sig)), 'signature changed'),
        (_with(event, sig: other.sig), 'signature from another event'),
        (_with(event, sig: event.sig.toUpperCase()), 'uppercase signature'),
        (_with(event, id: other.id, sig: other.sig), 'id and sig of another'),
        (_with(event, sig: ''), 'empty signature'),
      ];
      for (final (broken, label) in cases) {
        expect(
          verifySavedMessageEvent(broken, relayAuthor: _relay),
          isFalse,
          reason: 'kind $kind: $label',
        );
      }
    }
  });

  test('a relay-authored kind signed by another key is refused even when '
      'its proof claims the relay key', () {
    final byUser = _signed(39005, _userSecret);
    expect(
      verifySavedMessageEvent(
        _with(byUser, pubkey: _relay),
        relayAuthor: _relay,
      ),
      isFalse,
    );
  });
}
