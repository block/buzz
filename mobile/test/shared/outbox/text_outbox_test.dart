import 'dart:io';

import 'package:buzz/shared/outbox/text_outbox_delivery.dart';
import 'package:buzz/shared/outbox/text_outbox_store.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:path/path.dart' as p;
import 'package:sqlite3/sqlite3.dart';

const _scope = (communityId: 'first-community', pubkey: 'alice');
const _otherCommunity = (communityId: 'other-community', pubkey: 'alice');
const _otherSigner = (communityId: 'first-community', pubkey: 'bob');

TextOutboxDraft _draft(String text) => (
  channelId: 'channel',
  parentId: 'parent',
  rootId: 'root',
  text: text,
  content: text,
  recipients: const [],
  tags: const [
    ['mention', 'reference'],
  ],
  personal: false,
  implicitSoloAgent: null,
);

void main() {
  late Directory directory;
  late TextOutboxStore store;

  setUp(() async {
    directory = await Directory.systemTemp.createTemp('text-outbox-test');
    store = TextOutboxStore(directory: () async => directory);
  });

  tearDown(() async {
    store.close();
    await directory.delete(recursive: true);
  });

  NostrEvent sign(TextOutboxMessage row, int timestamp) => NostrEvent(
    id: '${row.id}-${timestamp ~/ 1000}',
    pubkey: _scope.pubkey,
    createdAt: timestamp ~/ 1000,
    kind: EventKind.streamMessage,
    tags: [
      ['h', row.draft.channelId],
      ...row.draft.tags,
    ],
    content: row.draft.content,
    sig: 'signature',
  );

  test(
    'offline text survives restart and first signs in compose order',
    () async {
      var now = DateTime.fromMillisecondsSinceEpoch(4000000);
      await store.enqueue(_scope, 'one', _draft('keep this text'));
      await store.enqueue(_scope, 'one', _draft('keep this text'));
      await store.enqueue(_scope, 'two', _draft('keep this text'));
      await store.enqueue(_scope, 'three', _draft('last'));
      store.close();
      store = TextOutboxStore(directory: () async => directory);
      now = now.add(const Duration(minutes: 16));
      final beforeConnect = await store.list(_scope);
      expect(beforeConnect.map((row) => row.id), ['one', 'two', 'three']);
      expect(beforeConnect.every((row) => row.signedEvent == null), isTrue);
      expect(beforeConnect.first.draft.recipients, isEmpty);
      expect(beforeConnect.first.draft.tags, _draft('unused').tags);
      final published = <NostrEvent>[];

      final delivery = TextOutboxDelivery(
        store: store,
        releaseDraft: (_) async => true,
        prepare: (row, timestamp) async => sign(row, timestamp()),
        publish: (event, {required beforeWire, required validateScope}) async {
          final row = (await store.list(_scope)).first;
          expect(row.signedEvent?.toJson(), event.toJson());
          expect(row.mayHaveSent, isFalse);
          expect(await beforeWire(), isTrue);
          validateScope();
          expect((await store.list(_scope)).first.mayHaveSent, isTrue);
          published.add(event);
        },
        lookup: (_) async => false,
        showAccepted: (_, _) {},
        now: () => now,
      );

      expect(await delivery.drain(_scope, () => true), isFalse);
      expect(published.map((event) => event.createdAt).toSet(), {
        now.millisecondsSinceEpoch ~/ 1000,
      });
      expect(published.map((event) => event.id.split('-').first), [
        'one',
        'two',
        'three',
      ]);
      expect(await store.list(_scope), isEmpty);
    },
  );

  for (final scenario in ['retry', 'found', 'empty']) {
    test('lost acknowledgement across restart: $scenario', () async {
      var now = DateTime.fromMillisecondsSinceEpoch(7000000);
      var signatures = 0;
      final attempted = <NostrEvent>[];
      final accepted = <String>[];
      final lookups = <String>[];
      await store.enqueue(_scope, 'send', _draft('recoverable message'));

      TextOutboxDelivery delivery() => TextOutboxDelivery(
        store: store,
        releaseDraft: (_) async => true,
        prepare: (row, timestamp) async {
          signatures += 1;
          return sign(row, timestamp());
        },
        publish: (event, {required beforeWire, required validateScope}) async {
          expect(await beforeWire(), isTrue);
          validateScope();
          attempted.add(event);
          if (attempted.length == 1) throw RelayAckLostError();
        },
        lookup: (event) async {
          lookups.add(event.id);
          return scenario == 'found';
        },
        showAccepted: (_, event) => accepted.add(event.id),
        now: () => now,
      );

      expect(await delivery().drain(_scope, () => true), isTrue);
      final first = attempted.single;
      store.close();
      store = TextOutboxStore(directory: () async => directory);
      if (scenario != 'retry') now = now.add(const Duration(minutes: 16));

      expect(await delivery().drain(_scope, () => true), isFalse);
      final remaining = await store.list(_scope);
      expect(signatures, 1);
      expect(attempted.map((event) => event.toJson()), [
        first.toJson(),
        if (scenario == 'retry') first.toJson(),
      ]);
      expect(lookups, [if (scenario != 'retry') first.id]);
      expect(accepted, [if (scenario != 'empty') first.id]);
      if (scenario == 'empty') {
        final uncertain = remaining.single;
        expect(uncertain.status, TextOutboxStatus.unconfirmed);
        expect(uncertain.draft.text, 'recoverable message');
        await store.retry(_scope, uncertain.id);
        await delivery().drain(_scope, () => true);
        expect(signatures, 1);
        expect(attempted, [first]);
        expect(
          (await store.list(_scope)).single.status,
          TextOutboxStatus.unconfirmed,
        );
        await store.discard(_scope, uncertain.id);
        expect(await store.list(_scope), isEmpty);
      } else {
        expect(remaining, isEmpty);
      }
    });
  }

  for (final priorLostAck in [false, true]) {
    for (final rejection in ['blocked: archived', 'invalid: event timestamp']) {
      test(
        'retry after expiry, prior lost ack $priorLostAck, $rejection',
        () async {
          var now = DateTime.fromMillisecondsSinceEpoch(8000000);
          final wire = <NostrEvent>[];
          final lookups = <String>[];
          final accepted = <String>[];
          var rejected = false;
          await store.enqueue(_scope, 'send', _draft('keep my message'));

          TextOutboxDelivery delivery() => TextOutboxDelivery(
            store: store,
            releaseDraft: (_) async => true,
            prepare: (row, timestamp) async => sign(row, timestamp()),
            publish:
                (event, {required beforeWire, required validateScope}) async {
                  expect(await beforeWire(), isTrue);
                  validateScope();
                  wire.add(event);
                  if (priorLostAck && wire.length == 1) {
                    throw RelayAckLostError();
                  }
                  if (!rejected) {
                    rejected = true;
                    throw RelayEventRejectedException(rejection);
                  }
                },
            lookup: (event) async {
              lookups.add(event.id);
              return true;
            },
            showAccepted: (_, event) => accepted.add(event.id),
            now: () => now,
          );

          if (priorLostAck) {
            expect(await delivery().drain(_scope, () => true), isTrue);
          }
          expect(await delivery().drain(_scope, () => true), isFalse);
          final failed = (await store.list(_scope)).single;
          expect(failed.status, TextOutboxStatus.failed);
          expect(failed.mayHaveSent, priorLostAck);
          expect(failed.draft.text, 'keep my message');
          final original = wire.first.id;
          expect(wire.every((event) => event.id == original), isTrue);

          store.close();
          store = TextOutboxStore(directory: () async => directory);
          now = now.add(const Duration(minutes: 16));
          await store.retry(_scope, failed.id);
          expect(await delivery().drain(_scope, () => true), isFalse);
          expect(lookups, priorLostAck ? [original] : isEmpty);
          if (priorLostAck) {
            expect(wire.map((event) => event.id), [original, original]);
            expect(accepted, [original]);
          } else {
            expect(wire, hasLength(2));
            expect(wire.last.id, isNot(original));
            expect(accepted, [wire.last.id]);
          }
          expect(await store.list(_scope), isEmpty);
        },
      );
    }
  }

  test('member fetch failure preserves unsigned text until retry', () async {
    await store.enqueue(_scope, 'one', _draft('mention was settled'));
    var membersAvailable = false;
    final delivery = TextOutboxDelivery(
      store: store,
      releaseDraft: (_) async => true,
      prepare: (row, timestamp) async {
        if (!membersAvailable) throw RelayNotConnectedError();
        return sign(row, timestamp());
      },
      publish: (_, {required beforeWire, required validateScope}) async {
        expect(await beforeWire(), isTrue);
        validateScope();
      },
      lookup: (_) async => false,
      showAccepted: (_, _) {},
    );
    expect(await delivery.drain(_scope, () => true), isTrue);
    final row = (await store.list(_scope)).single;
    expect(row.status, TextOutboxStatus.pending);
    expect(row.signedEvent, isNull);
    membersAvailable = true;
    expect(await delivery.drain(_scope, () => true), isFalse);
    expect(await store.list(_scope), isEmpty);
  });

  test(
    'removal fences writes and startup drops orphaned signing scopes',
    () async {
      await store.enqueue(_scope, 'first', _draft('will be removed'));
      await store.enqueue(_otherSigner, 'wrong-signer', _draft('orphan'));
      await store.enqueue(_otherCommunity, 'other', _draft('keep'));
      await store.removeOrphans({_scope, _otherCommunity});
      expect(await store.list(_otherSigner), isEmpty);

      expect(await store.hasMessages(communityId: _scope.communityId), isTrue);
      store.beginRemoval(_scope.communityId);
      store.beginRemoval(_scope.communityId);
      store.endRemoval(_scope.communityId);
      expect(store.isRetiring(_scope.communityId), isTrue);
      await expectLater(
        store.enqueue(_scope, 'late', _draft('late')),
        throwsStateError,
      );
      await store.removeCommunity(_scope.communityId);
      store.endRemoval(_scope.communityId);
      expect(await store.list(_scope), isEmpty);
      expect((await store.list(_otherCommunity)).single.draft.text, 'keep');
    },
  );

  test('version one attempt history stays uncertain after upgrade', () async {
    await store.enqueue(_scope, 'one', _draft('saved before upgrading'));
    final original = sign((await store.list(_scope)).single, 1000);
    await store.saveSignature(_scope, 'one', original);
    await store.markAttempted(_scope, 'one');
    final database = sqlite3.open(p.join(directory.path, 'outbox.sqlite'));
    database.userVersion = 1;
    final signedBytes = database
        .select('SELECT signed_event FROM outbox')
        .single['signed_event'];
    database.close();

    final reopened = (await store.list(_scope)).single;
    expect(reopened.signedEvent?.toJson(), original.toJson());
    await store.recordRejection(
      _scope,
      reopened.id,
      status: TextOutboxStatus.failed,
      definitive: true,
    );
    expect((await store.list(_scope)).single.mayHaveSent, isTrue);
    final upgraded = sqlite3.open(p.join(directory.path, 'outbox.sqlite'));
    expect(upgraded.userVersion, 2);
    expect(
      upgraded.select('SELECT signed_event FROM outbox').single['signed_event'],
      signedBytes,
    );
    upgraded.close();
  });

  test('a newer schema retains the database and its original text', () async {
    await store.enqueue(_scope, 'first', _draft('keep this'));
    final database = sqlite3.open(p.join(directory.path, 'outbox.sqlite'));
    database.userVersion = 3;
    database.close();
    await expectLater(store.list(_scope), throwsStateError);

    final reopened = sqlite3.open(p.join(directory.path, 'outbox.sqlite'));
    expect(reopened.userVersion, 3);
    expect(
      reopened.select('SELECT text FROM outbox').single['text'],
      'keep this',
    );
    reopened.close();
  });
}
