import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/shared/auth/federated_identity.dart';

void main() {
  final keys = nostr.Keys('1'.padLeft(64, '0'));
  final nsec = keys.nsec;
  final now = DateTime.utc(2026, 9, 15);

  test('enterprise admission stays scoped and expires at equality', () {
    final session = FederatedIdentitySession(
      origins: ['https://relay.example'],
    );
    expect(
      () => session.headers('wss://relay.example', nsec, now: now),
      throwsStateError,
    );
    session.install(
      generation: 0,
      jwt: 'aaa.bbb.ccc',
      pubkey: keys.public,
      expires: now.add(const Duration(seconds: 60)),
      now: now,
    );
    expect(
      session.headers('wss://relay.example/huddle/x/audio', nsec, now: now),
      {'Nostr-Federated-Identity': 'Bearer aaa.bbb.ccc'},
    );
    expect(
      session.headers('https://other.example/media/x', nsec, now: now),
      isEmpty,
    );
    expect(
      session.headers('https://relay.example:444/query', nsec, now: now),
      isEmpty,
    );
    expect(
      () => session.headers('https://relay.example/query', null, now: now),
      throwsStateError,
    );
    expect(
      () => session.headers(
        'https://relay.example/query',
        nsec,
        now: now.add(const Duration(seconds: 60)),
      ),
      throwsStateError,
    );
  });

  test('logout rejects stale completion without exposing assertion', () {
    final session = FederatedIdentitySession(
      origins: ['https://relay.example'],
    );
    final generation = session.generation;
    session.invalidate();
    expect(
      () => session.install(
        generation: generation,
        jwt: 'aaa.bbb.ccc',
        pubkey: keys.public,
        expires: now.add(const Duration(minutes: 1)),
        now: now,
      ),
      throwsStateError,
    );
    expect(session.toString(), isNot(contains('aaa.bbb.ccc')));
  });

  test('OSS does not need enterprise evidence', () {
    expect(
      FederatedIdentitySession(origins: []).headers('ws://fixture', null),
      isEmpty,
    );
  });
}
