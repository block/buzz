import 'dart:async';

import 'package:flutter_test/flutter_test.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:buzz/shared/auth/event_signer.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:buzz/shared/relay/relay_session.dart';
import 'package:buzz/shared/relay/signed_event_relay.dart';

final class _DeferredSigner implements EventSigner {
  final LocalEventSigner local;
  final ready = Completer<void>();
  UnsignedEvent? captured;
  _DeferredSigner(this.local);
  @override
  String get publicKey => local.publicKey;
  @override
  Future<nostr.Event> sign(UnsignedEvent event) async {
    captured = event;
    await ready.future;
    return local.sign(event);
  }
}

final class _Session extends RelaySessionNotifier {
  NostrEvent? published;
  bool wrongAck = false;
  @override
  Future<NostrEvent> publish(
    NostrEvent event, {
    required RelaySessionLease lease,
    Duration timeout = const Duration(seconds: 8),
  }) async {
    published = event;
    return wrongAck
        ? NostrEvent.fromJson({...event.toJson(), 'id': '0' * 64})
        : event;
  }
}

void main() {
  test('submission rejects an acknowledgement for a different event', () async {
    final session = _Session()..wrongAck = true;
    final relay = SignedEventRelay.withSigner(
      session: session,
      signer: LocalEventSigner(nostr.Keys.generate().secret),
    );
    await expectLater(
      relay.submit(kind: 1, content: 'exact', tags: []),
      throwsStateError,
    );
    expect(session.published, isNotNull);
  });

  test('local signer preserves the legacy event id and exact fields', () async {
    final keys = nostr.Keys.generate();
    final signer = LocalEventSigner(keys.secret);
    final tags = [
      ['h', 'channel'],
      ['x', 'one', 'two'],
    ];
    final unsigned = UnsignedEvent(
      publicKey: signer.publicKey,
      createdAt: 1700000000,
      kind: 40002,
      content: 'exact 🐝\ncontent',
      tags: tags,
    );
    final legacy = nostr.Event.from(
      kind: unsigned.kind,
      content: unsigned.content,
      tags: tags,
      createdAt: unsigned.createdAt,
      secretKey: keys.secret,
    );
    final signed = await signer.sign(unsigned);
    expect(signed.id, legacy.id);
    expect(signed.pubkey, legacy.pubkey);
    expect(signed.tags, legacy.tags);
    expect(signed.content, legacy.content);
    expect(signed.createdAt, legacy.createdAt);
    expect(signed.kind, legacy.kind);
    expect(signed.isValid(), isTrue);
    tags.first[1] = 'changed';
    expect(unsigned.tags.first[1], 'channel');
    expect(() => unsigned.tags.first.add('mutation'), throwsUnsupportedError);
  });

  test('wrong author is rejected, never rewritten', () async {
    final signer = LocalEventSigner(nostr.Keys.generate().secret);
    await expectLater(
      signer.sign(
        UnsignedEvent(
          publicKey: nostr.Keys.generate().public,
          createdAt: 1700000000,
          kind: 1,
          content: '',
          tags: [],
        ),
      ),
      throwsStateError,
    );
  });

  test(
    'submission awaits signing and preserves its captured payload',
    () async {
      final signer = _DeferredSigner(
        LocalEventSigner(nostr.Keys.generate().secret),
      );
      final session = _Session();
      final relay = SignedEventRelay.withSigner(
        session: session,
        signer: signer,
      );
      final tags = [
        ['h', 'channel'],
      ];
      NostrEvent? callback;
      final pending = relay.submit(
        kind: 40002,
        content: 'pending',
        tags: tags,
        createdAt: 1700000000,
        onSigned: (event) => callback = event,
      );
      tags.first[1] = 'mutated while signing';
      expect(session.published, isNull);
      expect(callback, isNull);
      expect(signer.captured!.tags, [
        ['h', 'channel'],
      ]);
      signer.ready.complete();
      final result = await pending;
      expect(identical(result, session.published), isTrue);
      expect(identical(result, callback), isTrue);
      expect(result.tags, [
        ['h', 'channel'],
      ]);
      expect(result.createdAt, 1700000000);
    },
  );

  test(
    'signing failure propagates without publication or a local fallback',
    () async {
      final signer = _DeferredSigner(
        LocalEventSigner(nostr.Keys.generate().secret),
      );
      final session = _Session();
      final relay = SignedEventRelay(
        session: session,
        nsec: nostr.Keys.generate().nsec,
        signer: signer,
      );
      final pending = relay.submit(kind: 1, content: '', tags: []);
      final assertion = expectLater(pending, throwsStateError);
      signer.ready.completeError(StateError('sign failed'));
      await assertion;
      expect(session.published, isNull);
    },
  );
}
