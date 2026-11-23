import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/channels/saved_message_store.dart';
import 'package:buzz/shared/crypto/event_signing.dart';
import 'package:buzz/shared/diagnostics/diagnostic_reporter.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:sqlite3/sqlite3.dart';

const _identity = (communityId: 'community-one', pubkey: 'viewer-one');
const _otherIdentity = (communityId: 'community-one', pubkey: 'viewer-two');
const _otherCommunity = (communityId: 'community-two', pubkey: 'viewer-one');
const _channel = (channelId: 'channel-one', rootId: '');
const _otherChannel = (channelId: 'channel-two', rootId: '');
const _thirdChannel = (channelId: 'channel-three', rootId: '');
const _secret =
    '0000000000000000000000000000000000000000000000000000000000000001';

NostrEvent _message(
  SavedMessagePage page,
  String text, {
  int kind = EventKind.streamMessage,
  String? target,
}) => NostrEvent.fromJson(
  jsonDecode(
        signNostrEvent(
          kind: kind,
          content: text,
          secretKey: _secret,
          createdAt: 1700000000,
          tags: [
            ['h', page.channelId],
            if (target != null) ['e', target],
          ],
        ).toJson(),
      )
      as Map<String, dynamic>,
);

class _Diagnostics extends NoopDiagnosticReporter {
  final operations = <DiagnosticOperation>[];
  void Function()? onReport;

  @override
  void reportError(
    Object error,
    StackTrace? stackTrace, {
    required DiagnosticOperation operation,
  }) {
    operations.add(operation);
    onReport?.call();
  }
}

void main() {
  late Directory temp;
  late SavedMessageStore store;
  late _Diagnostics diagnostics;

  setUp(() async {
    temp = await Directory.systemTemp.createTemp('buzz-saved-message-test');
    diagnostics = _Diagnostics();
    store = SavedMessageStore(root: () async => temp, diagnostics: diagnostics);
  });

  tearDown(() async => temp.delete(recursive: true));

  test(
    'reopens the verified page and replaces it with a fresh result',
    () async {
      final original = _message(_channel, 'old message');
      final otherScope = _message(_otherChannel, 'other channel');
      final otherIdentity = _message(_channel, 'other identity');
      final tampered = NostrEvent.fromJson({
        ...original.toJson(),
        'content': 'tampered message',
      });
      final ticket = store.capture(_identity, _channel);
      await store.save(ticket, [
        tampered,
        otherScope,
        original,
      ], relayAuthor: null);
      await store.save(store.capture(_otherIdentity, _channel), [
        otherIdentity,
      ], relayAuthor: null);
      final reopened = SavedMessageStore(
        root: () async => temp,
        diagnostics: diagnostics,
      );
      expect(await reopened.read(reopened.capture(_identity, _channel)), [
        original,
      ]);
      expect(await reopened.read(reopened.capture(_otherIdentity, _channel)), [
        otherIdentity,
      ]);
      expect(
        await reopened.read(reopened.capture(_otherCommunity, _channel)),
        isNull,
      );

      final fresh = _message(_channel, 'new message');
      await store.save(ticket, [fresh], relayAuthor: null);
      expect(await reopened.read(reopened.capture(_identity, _channel)), [
        fresh,
      ]);
      await store.save(ticket, [], relayAuthor: null);
      expect(
        await reopened.read(reopened.capture(_identity, _channel)),
        isNull,
      );
      expect(diagnostics.operations, isEmpty);
    },
  );

  test('evicts the least recently opened page within the budget', () async {
    final first = _message(_channel, 'first');
    final second = _message(_otherChannel, 'second');
    final third = _message(_thirdChannel, 'third');
    final capped = SavedMessageStore(
      root: () async => temp,
      diagnostics: diagnostics,
      payloadBudget: 900,
    );
    final firstPage = capped.capture(_identity, _channel);
    final secondPage = capped.capture(_identity, _otherChannel);
    final thirdPage = capped.capture(_identity, _thirdChannel);
    await capped.save(firstPage, [first], relayAuthor: null);
    await capped.save(secondPage, [second], relayAuthor: null);
    await capped.touch(firstPage); // Reopen first from the warm timeline.
    await capped.save(thirdPage, [third], relayAuthor: null);
    expect(await capped.read(secondPage), isNull);
    expect(await capped.read(firstPage), [first]);
    expect(await capped.read(thirdPage), [third]);
  });

  test(
    'relay authority changes discard pages verified under the old key',
    () async {
      final message = _message(_channel, 'message with replies');
      final cached = store.capture(_identity, _channel);
      final other = store.capture(_identity, _otherChannel);
      final summary = NostrEvent.fromJson(
        signNostrEvent(
          kind: EventKind.channelThreadSummary,
          secretKey: _secret,
          content: '{"reply_count":1,"descendant_count":1}',
          createdAt: 1700000001,
          tags: [
            ['e', message.id],
            ['d', message.id],
            ['h', _channel.channelId],
          ],
        ).toMap(),
      );
      await store.save(cached, [message, summary], relayAuthor: summary.pubkey);
      expect(await store.read(cached), [message, summary]);

      final newAuthor = derivePublicKeyHex(
        '0000000000000000000000000000000000000000000000000000000000000002',
      );
      final newMessage = _message(_otherChannel, 'after rotation');
      await store.save(other, [newMessage], relayAuthor: newAuthor);
      expect(await store.read(cached), isNull);
      expect(await store.read(other), [newMessage]);
    },
  );

  test(
    'scope removal retires a queued writer and preserves other identities',
    () async {
      final root = Completer<Directory>();
      final started = Completer<void>();
      final waiting = SavedMessageStore(
        root: () {
          if (!started.isCompleted) started.complete();
          return root.future;
        },
        diagnostics: diagnostics,
      );
      final ticket = waiting.capture(_identity, _channel);
      final other = waiting.capture(_otherIdentity, _channel);
      final otherEvent = _message(_channel, 'other identity');
      final write = waiting.save(ticket, [
        _message(_channel, 'old'),
      ], relayAuthor: null);
      await started.future;
      final removed = waiting.removeScope(_identity);
      root.complete(temp);
      await Future.wait([write, removed]);
      await waiting.save(ticket, [
        _message(_channel, 'late'),
      ], relayAuthor: null);
      await waiting.save(other, [otherEvent], relayAuthor: null);
      expect(await waiting.read(waiting.capture(_identity, _channel)), isNull);
      expect(await waiting.read(other), [otherEvent]);
      expect(diagnostics.operations, isEmpty);
    },
  );

  test(
    'channel retirement purges all its windows and refuses retired tickets',
    () async {
      final page = store.capture(_identity, _channel);
      final thread = store.capture(_identity, (
        channelId: _channel.channelId,
        rootId: 'root',
      ));
      final other = store.capture(_identity, _otherChannel);
      final sameChannel = _message(_channel, 'channel body');
      final differentChannel = _message(_otherChannel, 'retained');
      await store.save(page, [sameChannel], relayAuthor: null);
      await store.save(thread, [sameChannel], relayAuthor: null);
      await store.save(other, [differentChannel], relayAuthor: null);
      await store.removeChannel(_identity, _channel.channelId);
      await store.save(page, [sameChannel], relayAuthor: null);
      expect(await store.read(store.capture(_identity, _channel)), isNull);
      expect(
        await store.read(
          store.capture(_identity, (
            channelId: _channel.channelId,
            rootId: 'root',
          )),
        ),
        isNull,
      );
      expect(await store.read(other), [differentChannel]);
      expect(diagnostics.operations, isEmpty);
    },
  );

  test('coalesces live removals while retaining other saved rows', () async {
    var rootReads = 0;
    final cache = SavedMessageStore(
      root: () async {
        rootReads++;
        return temp;
      },
      diagnostics: diagnostics,
    );
    final original = cache.capture(_identity, _channel);
    final first = _message(_channel, 'first removed row');
    final second = _message(_channel, 'second removed row');
    final retained = _message(_channel, 'unchanged');
    await cache.save(original, [first, second, retained], relayAuthor: null);
    final file = Directory(
      '${temp.path}/saved_messages',
    ).listSync().whereType<File>().single;
    expect(
      latin1.decode(file.readAsBytesSync()).contains(first.content),
      isTrue,
    );
    rootReads = 0;

    cache.removeChannelEvents(_identity, _channel.channelId, {first.id});
    cache.removeChannelEvents(_identity, _channel.channelId, {second.id});

    expect(cache.isCurrent(original), isFalse);
    expect(await cache.read(cache.capture(_identity, _channel)), [retained]);
    expect(
      latin1.decode(file.readAsBytesSync()).contains(first.content),
      isFalse,
      reason: 'Deleted content must not survive in the database file.',
    );
    expect(File('${file.path}-journal').existsSync(), isFalse);
    expect(rootReads, 2); // One removal batch and one read.
    expect(diagnostics.operations, isEmpty);
  });

  test('evicts old pages and retries a write at the physical cap', () async {
    const pageLimit = 24;
    final cache = SavedMessageStore(
      root: () async => temp,
      diagnostics: diagnostics,
      databasePageLimit: pageLimit,
    );
    final tickets = <SavedMessageTicket>[];
    for (var channel = 0; channel < 16; channel++) {
      final page = (channelId: 'capacity-channel-$channel', rootId: '');
      final ticket = cache.capture(_identity, page);
      tickets.add(ticket);
      final events = [
        for (var row = 0; row < 4; row++) _message(page, '$row ${'x' * 1500}'),
      ];
      await cache.save(ticket, events, relayAuthor: null);
      expect(await cache.read(ticket), events, reason: 'channel $channel');
    }

    expect(await cache.read(tickets.first), isNull);
    expect(await cache.read(tickets.last), isNotEmpty);
    expect(diagnostics.operations, isEmpty);
    final file = Directory(
      '${temp.path}/saved_messages',
    ).listSync().whereType<File>().single;
    expect(file.lengthSync(), lessThanOrEqualTo(pageLimit * 4096));
    final db = sqlite3.open(file.path, mode: OpenMode.readOnly);
    try {
      expect(db.select('PRAGMA journal_mode').single.values.single, 'delete');
    } finally {
      db.close();
    }
  });

  test('returns a readable page when its recency update fails', () async {
    final ticket = store.capture(_identity, _channel);
    final message = _message(_channel, 'still available offline');
    await store.save(ticket, [message], relayAuthor: null);
    final file = Directory(
      '${temp.path}/saved_messages',
    ).listSync().whereType<File>().single;
    final db = sqlite3.open(file.path);
    try {
      db.execute('''
        CREATE TRIGGER fail_recency_update BEFORE UPDATE OF opened_at ON pages
        BEGIN
          SELECT RAISE(ABORT, 'recency update failed');
        END;
      ''');
    } finally {
      db.close();
    }

    expect(await store.read(ticket), [message]);
    expect(diagnostics.operations, [DiagnosticOperation.storedData]);
  });

  test('omits a page that cannot fit its verified relay events', () async {
    final ticket = store.capture(_identity, _channel);
    final message = _message(_channel, 'saved history');
    await store.save(ticket, [message], relayAuthor: null);
    expect(await store.read(ticket), [message]);

    final rows = [
      for (var index = 0; index < 20; index++)
        _message(_channel, '$index ${'x' * (100 * 1024)}'),
    ];
    final edit = _message(
      _channel,
      'x' * (100 * 1024),
      kind: EventKind.streamMessageEdit,
      target: message.id,
    );
    await store.save(ticket, [message, ...rows, edit], relayAuthor: null);
    expect(await store.read(ticket), isNull);
  });

  test('discards the cache when a targeted SQL removal fails', () async {
    final page = store.capture(_identity, _channel);
    final message = _message(_channel, 'must be removed');
    await store.save(page, [message], relayAuthor: null);
    final file = Directory(
      '${temp.path}/saved_messages',
    ).listSync().whereType<File>().single;
    final db = sqlite3.open(file.path);
    try {
      db.execute('''
        CREATE TRIGGER fail_removal BEFORE DELETE ON page_events BEGIN
          SELECT RAISE(ABORT, 'deletion failed');
        END;
      ''');
    } finally {
      db.close();
    }
    final marker = File('${temp.path}/saved_message_purge');
    var markerWasPending = false;
    diagnostics.onReport = () => markerWasPending = marker.existsSync();

    store.removeChannelEvents(_identity, _channel.channelId, {message.id});

    expect(await store.read(store.capture(_identity, _channel)), isNull);
    expect(file.existsSync(), isFalse);
    expect(markerWasPending, isTrue);
    expect(marker.existsSync(), isFalse);
    expect(diagnostics.operations, [DiagnosticOperation.storedData]);
  });

  test('rebuilds after a purge was interrupted', () async {
    final ticket = store.capture(_identity, _channel);
    final other = store.capture(_otherIdentity, _channel);
    final original = _message(_channel, 'original page');
    await store.save(ticket, [original], relayAuthor: null);
    await store.save(other, [original], relayAuthor: null);

    final support = await Directory('${temp.path}/support').create();
    final marker = File('${support.path}/saved_message_purge');
    await marker.writeAsString('pending');
    final restarted = SavedMessageStore(
      root: () async => temp,
      purgeRoot: () async => support,
      diagnostics: diagnostics,
    );
    final page = restarted.capture(_identity, _channel);
    expect(await restarted.read(page), isNull);
    expect(marker.existsSync(), isTrue);

    final fresh = _message(_channel, 'fresh relay result');
    await restarted.save(page, [fresh], relayAuthor: null);
    expect(await restarted.read(page), [fresh]);
    expect(
      await restarted.read(restarted.capture(_otherIdentity, _channel)),
      isNull,
    );
    expect(marker.existsSync(), isFalse);
    expect(diagnostics.operations, isEmpty);
  });

  test('keeps the restart marker until both queued purges finish', () async {
    final first = Completer<void>();
    final second = Completer<void>();
    final secondStarted = Completer<void>();
    var opens = 0;
    final queued = SavedMessageStore(
      root: () async {
        switch (++opens) {
          case 1:
            await first.future;
          case 2:
            secondStarted.complete();
            await second.future;
        }
        return temp;
      },
      purgeRoot: () async => temp,
      diagnostics: diagnostics,
    );
    final marker = File('${temp.path}/saved_message_purge');
    try {
      await queued.removeChannel(
        _identity,
        _channel.channelId,
        deferDisk: true,
      );
      await queued.removeChannel(
        _identity,
        _otherChannel.channelId,
        deferDisk: true,
      );
      expect(marker.existsSync(), isTrue);
      first.complete();
      await secondStarted.future;
      expect(marker.existsSync(), isTrue);
    } finally {
      if (!first.isCompleted) first.complete();
      second.complete();
      await queued.read(queued.capture(_identity, _channel));
    }
    expect(marker.existsSync(), isFalse);
  });

  test(
    'discards an unreadable or incompatible file and refills on the next save',
    () async {
      final ticket = store.capture(_identity, _channel);
      final message = _message(_channel, 'refill');
      await store.save(ticket, [message], relayAuthor: null);
      final file = Directory(
        '${temp.path}/saved_messages',
      ).listSync().whereType<File>().single;
      final database = sqlite3.open(file.path);
      database.execute('PRAGMA user_version = 1');
      database.close();
      expect(await store.read(ticket), isNull);
      expect(file.existsSync(), isFalse);

      await file.writeAsString('broken cache');
      expect(await store.read(ticket), isNull);
      expect(file.existsSync(), isFalse);
      expect(diagnostics.operations, [
        DiagnosticOperation.storedData,
        DiagnosticOperation.storedData,
      ]);

      await store.save(ticket, [message], relayAuthor: null);
      expect(await store.read(ticket), [message]);
    },
  );
}
