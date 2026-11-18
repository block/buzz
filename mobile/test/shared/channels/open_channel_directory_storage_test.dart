import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/channels/open_channel_directory_storage.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';

NostrEvent _meta(String id, int createdAt) => NostrEvent(
  id: 'event-$id',
  pubkey: 'relay',
  createdAt: createdAt,
  kind: 39000,
  tags: [
    ['d', id],
    ['name', 'channel $id'],
  ],
  content: '',
  sig: 'signature',
);

void main() {
  late Directory root;

  setUp(() async {
    root = await Directory.systemTemp.createTemp('open-channel-directory');
    addTearDown(() => root.delete(recursive: true));
  });

  File fileFor(String community, String signer) =>
      OpenChannelDirectoryStorage.fileFor(root, community, signer);

  test('a saved directory reads back without signatures', () async {
    final file = fileFor('community', 'ABC');
    final storage = OpenChannelDirectoryStorage(Future.value(file));

    expect(await storage.read(), isNull);
    expect(
      await storage.write(
        SavedOpenChannelDirectory(
          metas: [_meta('a', 10), _meta('b', 20)],
          fullAt: 99,
          readAt: 98,
        ),
      ),
      isTrue,
    );

    expect(file.path, endsWith('channel-directory/abc.json'));
    expect(await file.readAsString(), isNot(contains('signature')));
    final saved = await OpenChannelDirectoryStorage(Future.value(file)).read();
    expect(saved!.fullAt, 99);
    expect(saved.readAt, 98);
    expect(
      saved.metas.map(
        (e) => [e.id, e.pubkey, e.createdAt, e.kind, e.tags, e.content],
      ),
      [
        [
          'event-a',
          'relay',
          10,
          39000,
          [
            ['d', 'a'],
            ['name', 'channel a'],
          ],
          '',
        ],
        [
          'event-b',
          'relay',
          20,
          39000,
          [
            ['d', 'b'],
            ['name', 'channel b'],
          ],
          '',
        ],
      ],
    );
  });

  test('unusable saved data is discarded', () async {
    final file = fileFor('community', 'abc');
    await file.parent.create(recursive: true);
    for (final raw in [
      'not json',
      jsonEncode({'v': 2, 'full_at': 1, 'metas': []}),
      jsonEncode({
        'v': 1,
        'full_at': 1,
        'metas': [
          {'id': 'x'},
        ],
      }),
    ]) {
      await file.writeAsString(raw);
      expect(
        await OpenChannelDirectoryStorage(Future.value(file)).read(),
        isNull,
      );
      expect(file.existsSync(), isFalse);
    }
  });

  test(
    'a directory over the bounds replaces the saved copy with none',
    () async {
      final file = fileFor('community', 'abc');
      final storage = OpenChannelDirectoryStorage(Future.value(file));
      await storage.write(
        SavedOpenChannelDirectory(metas: [_meta('a', 1)], fullAt: 1, readAt: 1),
      );

      final written = await storage.write(
        SavedOpenChannelDirectory(
          metas: [
            for (var i = 0; i <= maxSavedDirectoryChannels; i++) _meta('$i', 1),
          ],
          fullAt: 2,
          readAt: 2,
        ),
      );

      expect(written, isFalse);
      expect(file.existsSync(), isFalse);
    },
  );

  test('a closed store neither reads nor writes', () async {
    final file = fileFor('community', 'abc');
    final storage = OpenChannelDirectoryStorage(Future.value(file));
    final pending = storage.write(
      SavedOpenChannelDirectory(metas: [_meta('a', 1)], fullAt: 1, readAt: 1),
    );
    storage.close();

    expect(await pending, isFalse);
    expect(file.existsSync(), isFalse);
    expect(
      file.parent.existsSync() && file.parent.listSync().isNotEmpty,
      isFalse,
    );
    expect(await storage.read(), isNull);
  });

  test(
    'cleanup keeps only each community\'s current account and its save',
    () async {
      Future<void> save(String community, String signer) =>
          OpenChannelDirectoryStorage(
            Future.value(fileFor(community, signer)),
          ).write(
            SavedOpenChannelDirectory(
              metas: [_meta('a', 1)],
              fullAt: 1,
              readAt: 1,
            ),
          );
      await save('one', 'current');
      await save('one', 'signed-out');
      await save('two', 'current');
      // A save of the retained account in progress, and a stale one.
      final pending = File('${fileFor('one', 'current').path}.pending')
        ..writeAsStringSync('');
      final stalePending = File('${fileFor('one', 'signed-out').path}.pending')
        ..writeAsStringSync('');

      await OpenChannelDirectoryStorage.removeOrphans(
        root,
        () => {'one': 'current', 'two': null},
      );

      expect(fileFor('one', 'current').existsSync(), isTrue);
      expect(fileFor('one', 'signed-out').existsSync(), isFalse);
      expect(fileFor('two', 'current').existsSync(), isFalse);
      expect(pending.existsSync(), isTrue);
      expect(stalePending.existsSync(), isFalse);
    },
  );
}
