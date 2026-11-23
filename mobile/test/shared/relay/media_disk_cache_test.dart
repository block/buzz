import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:buzz/shared/relay/media_auth.dart';
import 'package:buzz/shared/relay/media_disk_cache.dart';
import 'package:buzz/shared/relay/media_image.dart';
import 'package:flutter/painting.dart';
import 'package:flutter_cache_manager/flutter_cache_manager.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:path/path.dart' as p;

// Minimal valid 1x1 transparent PNG.
final _pngBytes = base64Decode(
  'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNkYAAA'
  'AAYAAjCB0C8AAAAASUVORK5CYII=',
);

const _relayBase = 'https://relay.example.com';
const _hash =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _relayUrl = '$_relayBase/media/$_hash.png';
const _thumbnailUrl = '$_relayBase/media/$_hash.thumb.jpg';
const _largeUrl =
    '$_relayBase/media/'
    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb.png';
const _corruptUrl =
    '$_relayBase/media/'
    'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc.png';
const _truncatedUrl =
    '$_relayBase/media/'
    'dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd.png';
const _externalUrls = [
  'https://images.example.org/a.png',
  'https://images.example.org/b.png',
  'https://images.example.org/c.png',
];

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();

  late Directory root;
  final auth = MediaGetAuthService(baseUrl: _relayBase, nsec: null);

  setUp(() async {
    root = await Directory.systemTemp.createTemp('media_disk_cache_test');
    MediaImageProvider.debugResetCooldowns();
    PaintingBinding.instance.imageCache.clear();
    PaintingBinding.instance.imageCache.clearLiveImages();
  });

  tearDown(() => root.delete(recursive: true));

  test('images load from disk after a relaunch, and only stale external '
      'ones are refetched', () async {
    final fetched = <String>[];
    var online = true;
    // While set, external images answer once it completes, with a large
    // image.
    Completer<void>? replaceExternal;
    // While set, external images answer with a body that never decodes.
    var malformedExternal = false;
    // Trailing bytes after IEND still decode, so this is a 12 MiB image, the
    // size of a full-resolution phone photo.
    final largeBytes = [..._pngBytes, ...Uint8List(12 * 1024 * 1024)];
    final client = http_testing.MockClient((request) async {
      final url = request.url.toString();
      fetched.add(url);
      if (!online) throw const SocketException('offline');
      if (url == _largeUrl) return http.Response.bytes(largeBytes, 200);
      if (malformedExternal && _externalUrls.contains(url)) {
        return http.Response('not an image', 200);
      }
      if (replaceExternal != null && _externalUrls.contains(url)) {
        await replaceExternal.future;
        return http.Response.bytes(largeBytes, 200);
      }
      if (request.url.toString() == _corruptUrl) {
        return http.Response('not an image', 200);
      }
      // A valid header, so a codec opens, but the frame fails to decode.
      if (request.url.toString() == _truncatedUrl) {
        return http.Response.bytes(_pngBytes.sublist(0, 41), 200);
      }
      return http.Response.bytes(_pngBytes, 200);
    });
    final directory = MediaDiskCache.directoryFor(
      root,
      'community-1',
      'signer-1',
    );
    var now = DateTime.now();

    Future<void> launch(
      Future<void> Function(MediaDiskCache cache) body,
    ) async {
      // The caches root resolves after the first images start loading.
      final cache = MediaDiskCache.open(
        Future<Directory>.delayed(
          const Duration(milliseconds: 20),
          () => directory,
        ),
        now: () => now,
      );
      try {
        await body(cache);
      } finally {
        await cache.dispose();
        // A relaunch starts with nothing decoded in memory.
        PaintingBinding.instance.imageCache.clear();
        PaintingBinding.instance.imageCache.clearLiveImages();
      }
    }

    Future<Object?> show(MediaDiskCache cache, String url) => _waitError(
      MediaImageProvider(
        url: url,
        auth: auth,
        client: client,
        diskCache: cache,
      ).resolve(ImageConfiguration.empty),
    );

    const saved = [_relayUrl, _thumbnailUrl, _largeUrl, ..._externalUrls];

    // First launch downloads everything and saves what decodes.
    await launch((cache) async {
      for (final url in saved) {
        expect(await show(cache, url), isNull, reason: url);
      }
      expect(await show(cache, _corruptUrl), isNotNull);
      expect(await show(cache, _truncatedUrl), isNotNull);
      await _settle();
    });
    expect(fetched, [...saved, _corruptUrl, _truncatedUrl]);

    // Offline relaunch: the original and its thumbnail are separate entries,
    // both shown from disk, as is the large image. The bodies that failed to
    // decode were never saved.
    fetched.clear();
    online = false;
    await launch((cache) async {
      for (final url in saved) {
        expect(await show(cache, url), isNull, reason: url);
      }
      expect(await show(cache, _corruptUrl), isNotNull);
      expect(await show(cache, _truncatedUrl), isNotNull);
    });
    expect(fetched, [_corruptUrl, _truncatedUrl]);

    // A week on, external images show from disk at once and refresh in the
    // background, two at a time. Relay media never goes stale.
    fetched.clear();
    online = true;
    now = now.add(const Duration(days: 8));
    final replacement = replaceExternal = Completer<void>();
    await launch((cache) async {
      for (final url in saved) {
        expect(await show(cache, url), isNull, reason: url);
      }
      await _settle();
      expect(fetched, _externalUrls.take(2));
      // The third waits for a slot. The replacements are saved in place of
      // the old copies.
      replacement.complete();
      await _until(() async {
        for (final url in _externalUrls) {
          final hit = await cache.read(url);
          if (hit?.bytes.length != largeBytes.length) return false;
        }
        return true;
      });
      expect(fetched, _externalUrls);
    });
    replaceExternal = null;

    // A saved copy that no longer decodes is dropped and fetched again.
    fetched.clear();
    online = true;
    await launch((cache) async {
      await cache.write(
        _relayUrl,
        Uint8List.fromList(utf8.encode('not an image')),
        immutable: true,
      );
      expect(await show(cache, _relayUrl), isNotNull);
      await _settle();
      expect(await show(cache, _relayUrl), isNull);
      await cache.write(_externalUrls.first, _pngBytes, immutable: false);
    });
    expect(fetched, [_relayUrl]);

    // A refresh whose body never decodes backs off rather than refetching
    // on every load.
    fetched.clear();
    malformedExternal = true;
    // Forget the failures of the offline launch.
    MediaImageProvider.debugResetCooldowns();
    now = now.add(const Duration(days: 8));
    await launch((cache) async {
      for (var i = 0; i < 2; i++) {
        expect(await show(cache, _externalUrls.first), isNull);
        await _settle();
        PaintingBinding.instance.imageCache.clear();
      }
    });
    expect(fetched, [_externalUrls.first]);

    // A stale saved copy that fails to decode is dropped without starting a
    // refresh, which could otherwise save a new copy just before the drop.
    fetched.clear();
    malformedExternal = false;
    MediaImageProvider.debugResetCooldowns();
    await launch((cache) async {
      await cache.write(
        _externalUrls[1],
        Uint8List.fromList(utf8.encode('not an image')),
        immutable: false,
      );
    });
    now = now.add(const Duration(days: 8));
    await launch((cache) async {
      expect(await show(cache, _externalUrls[1]), isNotNull);
      await _settle();
      expect(fetched, isEmpty);
      expect(await show(cache, _externalUrls[1]), isNull);
    });
    expect(fetched, [_externalUrls[1]]);

    // Refreshes queued when the community or account retires never run.
    fetched.clear();
    malformedExternal = false;
    MediaImageProvider.debugResetCooldowns();
    await launch((cache) async {
      for (final url in _externalUrls) {
        await cache.write(url, _pngBytes, immutable: false);
      }
    });
    now = now.add(const Duration(days: 8));
    final held = replaceExternal = Completer<void>();
    await launch((cache) async {
      for (final url in _externalUrls) {
        expect(await show(cache, url), isNull, reason: url);
      }
      await _settle();
    });
    held.complete();
    await _settle();
    replaceExternal = null;
    expect(fetched, _externalUrls.take(2));

    // Removing the community deletes its directory.
    expect(await directory.exists(), isTrue);
    await MediaDiskCache.removeOrphans(root, () => {'community-1'});
    expect(await directory.exists(), isTrue);
    await MediaDiskCache.removeOrphans(root, () => {'community-2'});
    expect(
      await MediaDiskCache.communityDirectory(root, 'community-1').exists(),
      isFalse,
    );
  });

  test(
    'evicts least recently used files past the byte or file budget',
    () async {
      // (byte budget, file budget, evicted): three 4-byte files make 12 bytes.
      const cases = [
        (10, 100, ['old']),
        (100, 1, ['old', 'middle']),
      ];
      for (final (bytes, files, evicted) in cases) {
        final directory = MediaDiskCache.directoryFor(
          root,
          '$bytes-$files',
          's',
        );
        final cache = MediaDiskCache.open(
          directory,
          byteBudget: bytes,
          fileBudget: files,
          // Like the app's 10 s: long enough for the writes to finish.
          evictionDelay: const Duration(milliseconds: 200),
        );
        for (final name in ['old', 'middle', 'newest']) {
          await cache.write(name, Uint8List(4), immutable: true);
          // Distinct last-used times.
          await Future<void>.delayed(const Duration(milliseconds: 5));
        }
        // Saving a new URL schedules eviction, as in the app.
        await Future<void>.delayed(const Duration(milliseconds: 400));
        for (final name in ['old', 'middle', 'newest']) {
          expect(
            await cache.read(name),
            evicted.contains(name) ? isNull : isNotNull,
            reason: '$bytes bytes, $files files: $name',
          );
        }
        // Evicted files are gone from disk too; the index (and a save of it
        // in progress) remains.
        final names = directory
            .listSync()
            .map((entry) => p.basename(entry.path))
            .where((name) => !name.startsWith('index.json'));
        expect(names, hasLength(3 - evicted.length), reason: '$bytes $files');
        await cache.dispose();
      }

      Future<MediaDiskCache> open(String name) async => MediaDiskCache.open(
        MediaDiskCache.directoryFor(root, name, 's'),
        byteBudget: 10,
        evictionDelay: const Duration(milliseconds: 200),
      );
      const evictionRan = Duration(milliseconds: 400);

      // Reading a file from disk marks it used, even when the cache already
      // holds it in memory.
      final reads = await open('reads');
      await reads.write('old', Uint8List(4), immutable: true);
      await Future<void>.delayed(const Duration(milliseconds: 5));
      await reads.write('middle', Uint8List(4), immutable: true);
      await Future<void>.delayed(const Duration(milliseconds: 5));
      expect(await reads.read('old'), isNotNull);
      await reads.write('newest', Uint8List(4), immutable: true);
      await Future<void>.delayed(evictionRan);
      expect(await reads.read('middle'), isNull);
      expect(await reads.read('old'), isNotNull);
      await reads.dispose();

      // Replacing a file's bytes after eviction last ran, as a background
      // refresh does, evicts again once the cache is over budget.
      final refresh = await open('refresh');
      await refresh.write('other', Uint8List(4), immutable: true);
      await Future<void>.delayed(const Duration(milliseconds: 5));
      await refresh.write('refreshed', Uint8List(4), immutable: false);
      // Eviction ran with the cache in budget. A read would schedule it again.
      await Future<void>.delayed(evictionRan);
      await refresh.write('refreshed', Uint8List(8), immutable: false);
      await Future<void>.delayed(evictionRan);
      expect(await refresh.read('other'), isNull);
      expect((await refresh.read('refreshed'))?.bytes, hasLength(8));
      await refresh.dispose();

      // Overlapping saves of one URL, as when two sizes of one image decode
      // at once, keep one file on disk.
      final overlapping = await open('overlapping');
      await Future.wait([
        for (var i = 0; i < 2; i++)
          overlapping.write('same', Uint8List(4), immutable: true),
      ]);
      final files = MediaDiskCache.directoryFor(root, 'overlapping', 's')
          .listSync()
          .where((entry) => !p.basename(entry.path).startsWith('index.json'));
      expect(files, hasLength(1));
      await overlapping.dispose();

      // One eviction pass returns at most its limit, oldest first, and asks
      // for another pass.
      final indexDirectory = MediaDiskCache.directoryFor(root, 'index', 's')
        ..createSync(recursive: true);
      final index = MediaCacheIndex(
        File(p.join(indexDirectory.path, 'index.json')),
        indexDirectory,
        0,
        evictionsPerPass: 2,
      );
      await index.open();
      final epoch = DateTime(2026);
      for (var i = 0; i < 5; i++) {
        await index.insert(
          CacheObject(
            'url-$i',
            relativePath: 'file-$i',
            validTill: epoch,
            length: 1,
            touched: epoch.add(Duration(minutes: i)),
          ),
          setTouchedToNow: false,
        );
      }
      var morePasses = 0;
      index.onMoreToEvict = () => morePasses++;
      final firstPass = await index.getObjectsOverCapacity(100);
      expect(firstPass.map((o) => o.url), ['url-1', 'url-0']);
      expect(morePasses, 1);
      // A retired index's pass in progress records no deletions.
      final ids = [for (final o in await index.getAllObjects()) o.id!];
      index.retire();
      await index.deleteAll(ids);
      expect(await index.getAllObjects(), hasLength(5));
      await index.close();

      // Once closed, the index saves nothing more. A read that marks a file
      // used, or an eviction pass that finishes late, would otherwise save
      // it over the index a reopened cache wrote since.
      final indexFile = File(p.join(indexDirectory.path, 'index.json'));
      final reopenedIndex = MediaCacheIndex(indexFile, indexDirectory, 100);
      await reopenedIndex.open();
      await reopenedIndex.insert(
        CacheObject(
          'newer',
          relativePath: 'file-newer',
          validTill: epoch,
          length: 1,
        ),
      );
      await reopenedIndex.close();
      final written = indexFile.readAsStringSync();
      await index.updateOrInsert(
        CacheObject('late-read', relativePath: 'file-0', validTill: epoch),
      );
      await index.deleteAll(ids);
      await Future<void>.delayed(const Duration(milliseconds: 50));
      expect(indexFile.readAsStringSync(), written);

      // So a large save into a cache full of small files evicts them over
      // several passes, with no later reads to schedule them, until the
      // directory is back in budget.
      List<String> filesIn(Directory directory) => [
        for (final entry in directory.listSync())
          if (!p.basename(entry.path).startsWith('index.json'))
            p.basename(entry.path),
      ];
      final passesDirectory = MediaDiskCache.directoryFor(root, 'passes', 's');
      final passes = MediaDiskCache.open(
        passesDirectory,
        byteBudget: 20,
        evictionDelay: const Duration(milliseconds: 200),
        evictionsPerPass: 2,
      );
      for (var i = 0; i < 5; i++) {
        await passes.write('small-$i', Uint8List(4), immutable: true);
        await Future<void>.delayed(const Duration(milliseconds: 5));
      }
      await passes.write('large', Uint8List(16), immutable: true);
      await _until(() async => filesIn(passesDirectory).length == 2);
      expect(await passes.read('large'), isNotNull);
      expect(await passes.read('small-4'), isNotNull);
      await passes.dispose();

      // Files the index doesn't track, such as a save cut short by a full
      // disk or by the app exiting, are deleted on the next launch.
      final sweptDirectory = MediaDiskCache.directoryFor(root, 'swept', 's');
      final first = MediaDiskCache.open(sweptDirectory);
      await first.write('kept', Uint8List(4), immutable: true);
      await first.dispose();
      final earlier = DateTime.now().subtract(const Duration(minutes: 1));
      for (final name in ['untracked.file', 'interrupted.file.part']) {
        File(p.join(sweptDirectory.path, name))
          ..writeAsBytesSync(Uint8List(4))
          ..setLastModifiedSync(earlier);
      }
      // Closing waits for the sweep, so the next cache to open the directory
      // starts after it.
      final manyDirectory = MediaDiskCache.directoryFor(root, 'many', 's')
        ..createSync(recursive: true);
      for (var i = 0; i < 300; i++) {
        File(p.join(manyDirectory.path, 'orphan-$i.file'))
          ..writeAsBytesSync(Uint8List(1))
          ..setLastModifiedSync(earlier);
      }
      final manyIndex = MediaCacheIndex(
        File(p.join(manyDirectory.path, 'index.json')),
        manyDirectory,
        100,
      );
      await manyIndex.open();
      await manyIndex.close();
      expect(filesIn(manyDirectory), isEmpty);

      // The sweep runs in the background, and a save made while it runs
      // stays.
      // A part written just now may belong to a save in progress, so it
      // stays too.
      final fresh = File(p.join(sweptDirectory.path, 'fresh.file.part'))
        ..writeAsBytesSync(Uint8List(4));
      final relaunched = MediaDiskCache.open(sweptDirectory);
      await relaunched.write('during', Uint8List(4), immutable: true);
      expect(await relaunched.read('kept'), isNotNull);
      await _until(() async => filesIn(sweptDirectory).length == 3);
      expect(await relaunched.read('during'), isNotNull);
      expect(fresh.existsSync(), isTrue);
      await relaunched.dispose();

      // A disposed cache's scheduled eviction pass, running after the
      // community's cache was opened again, keeps the newer cache's saves.
      final retiredDirectory = MediaDiskCache.directoryFor(
        root,
        'retired',
        's',
      );
      Future<int> rows() async {
        final index = File(p.join(retiredDirectory.path, 'index.json'));
        return index.existsSync()
            ? (jsonDecode(await index.readAsString()) as List).length
            : 0;
      }

      const passDelay = Duration(milliseconds: 300);
      final retired = MediaDiskCache.open(
        retiredDirectory,
        byteBudget: 80,
        evictionDelay: passDelay,
        evictionsPerPass: 1,
      );
      for (var i = 0; i < 6; i++) {
        await retired.write('prior-$i', Uint8List(12), immutable: true);
        await Future<void>.delayed(const Duration(milliseconds: 5));
      }
      // Two files over budget: the first pass evicts one and schedules
      // another.
      await retired.write('large', Uint8List(24), immutable: true);
      await _until(() async => await rows() == 6);
      // As the provider does, the old cache is disposed with a save and a
      // read in progress, and not awaited before the reopen.
      final inFlight = retired.write(
        'in-flight',
        Uint8List(4),
        immutable: true,
      );
      final reading = retired.read('prior-3');
      await Future<void>.delayed(Duration.zero);
      final closing = retired.dispose();
      final reopened = MediaDiskCache.open(retiredDirectory, byteBudget: 80);
      await reopened.write('newer', Uint8List(4), immutable: true);
      await Future.wait([inFlight, reading, closing]);
      await reopened.dispose();
      await Future<void>.delayed(passDelay * 3);
      final relaunchedAgain = MediaDiskCache.open(
        retiredDirectory,
        byteBudget: 80,
      );
      expect(await relaunchedAgain.read('newer'), isNotNull);
      expect(await relaunchedAgain.read('in-flight'), isNotNull);
      // The old cache's next eviction would have taken it.
      expect(await relaunchedAgain.read('prior-1'), isNotNull);
      await relaunchedAgain.dispose();

      // A cache reopened while the previous one is slow to close goes
      // without the disk at first, then uses it once that close finishes.
      final slowDirectory = MediaDiskCache.directoryFor(root, 'slow', 's');
      final saving = MediaDiskCache.open(slowDirectory);
      await saving.write('saved', Uint8List(4), immutable: true);
      await saving.dispose();
      final gated = _GatedDirectory(slowDirectory);
      final slow = MediaDiskCache.open(gated);
      // Opens the index, whose sweep then waits on the gate, and so does
      // closing.
      expect(await slow.read('saved'), isNotNull);
      final slowSave = slow.write('slow-save', Uint8List(4), immutable: true);
      await Future<void>.delayed(Duration.zero);
      final slowClosing = slow.dispose();
      const openTimeout = Duration(milliseconds: 100);
      final waiting = MediaDiskCache.open(
        slowDirectory,
        openTimeout: openTimeout,
      );
      expect(await waiting.read('saved'), isNull);
      gated.release();
      await Future.wait([slowSave, slowClosing]);
      await Future<void>.delayed(Duration.zero);
      expect(await waiting.read('saved'), isNotNull);
      await waiting.write('after-close', Uint8List(4), immutable: true);
      await waiting.dispose();
      final afterSlow = MediaDiskCache.open(slowDirectory);
      expect(await afterSlow.read('after-close'), isNotNull);
      expect(await afterSlow.read('slow-save'), isNotNull);
      await afterSlow.dispose();

      // Reads during refreshes see the old bytes or the new ones in full,
      // never part of a file.
      final refreshing = MediaDiskCache.open(
        MediaDiskCache.directoryFor(root, 'refreshing', 's'),
      );
      const sizes = [1024, 8 * 1024 * 1024];
      await refreshing.write('image', Uint8List(sizes[0]), immutable: false);
      final lengths = <int?>{};
      Future<void> refreshRepeatedly() async {
        for (var i = 1; i <= 10; i++) {
          await refreshing.write(
            'image',
            Uint8List(sizes[i % 2]),
            immutable: false,
          );
        }
      }

      // A bounded number of reads: each one marks the file used, and a
      // reader that never stops keeps the index save, and so the refresh,
      // from finishing.
      Future<void> readRepeatedly() async {
        for (var i = 0; i < 20; i++) {
          lengths.add((await refreshing.read('image'))?.bytes.length);
        }
      }

      await Future.wait([
        refreshRepeatedly(),
        for (var i = 0; i < 3; i++) readRepeatedly(),
      ]);
      expect(lengths, everyElement(isIn(sizes)));
      await refreshing.dispose();
    },
  );

  test('encoded bytes for a native surface download once, across a '
      'relaunch, and bytes that never decode are not saved', () async {
    final fetched = <String>[];
    final client = http_testing.MockClient((request) async {
      fetched.add(request.url.toString());
      if (request.url.toString() == _corruptUrl) {
        return http.Response('not an image', 200);
      }
      return http.Response.bytes(_pngBytes, 200);
    });
    final directory = MediaDiskCache.directoryFor(
      root,
      'community-1',
      'signer-1',
    );
    Future<Uint8List> load(MediaDiskCache cache, String url) =>
        MediaImageProvider(
          url: url,
          auth: auth,
          client: client,
          diskCache: cache,
        ).loadEncodedBytes(cancel: Completer<void>().future);

    for (var launch = 0; launch < 2; launch++) {
      final cache = MediaDiskCache.open(directory);
      try {
        expect(await load(cache, _relayUrl), _pngBytes);
        await expectLater(load(cache, _corruptUrl), throwsA(anything));
        await _settle();
      } finally {
        await cache.dispose();
      }
      // A failed body goes on cooldown; let the next launch retry it.
      MediaImageProvider.debugResetCooldowns();
    }

    expect(fetched, [_relayUrl, _corruptUrl, _corruptUrl]);
  });

  test('encoded bytes remove a damaged saved copy so the next request '
      'downloads it again', () async {
    final fetched = <String>[];
    final client = http_testing.MockClient((request) async {
      fetched.add(request.url.toString());
      return http.Response.bytes(_pngBytes, 200);
    });
    final cache = MediaDiskCache.open(
      MediaDiskCache.directoryFor(root, 'community-1', 'signer-1'),
    );
    try {
      await cache.write(
        _truncatedUrl,
        Uint8List.fromList(_pngBytes.sublist(0, 20)),
        immutable: true,
      );
      Future<Uint8List> load() => MediaImageProvider(
        url: _truncatedUrl,
        auth: auth,
        client: client,
        diskCache: cache,
      ).loadEncodedBytes(cancel: Completer<void>().future);

      expect(await cache.contains(_truncatedUrl), isTrue);
      await expectLater(load(), throwsA(anything));
      expect(fetched, isEmpty);
      await _settle();
      expect(await cache.contains(_truncatedUrl), isFalse);
      MediaImageProvider.debugResetCooldowns();
      expect(await load(), _pngBytes);
      expect(fetched, [_truncatedUrl]);
    } finally {
      await cache.dispose();
    }
  });

  test(
    'encoded bytes share one download with simultaneous image loads',
    () async {
      final fetched = <String>[];
      final respond = Completer<void>();
      final client = http_testing.MockClient((request) async {
        fetched.add(request.url.toString());
        await respond.future;
        return http.Response.bytes(_pngBytes, 200);
      });
      final cache = MediaDiskCache.open(
        MediaDiskCache.directoryFor(root, 'community-1', 'signer-1'),
      );
      try {
        MediaImageProvider provider() => MediaImageProvider(
          url: _relayUrl,
          auth: auth,
          client: client,
          diskCache: cache,
        );
        final first = provider().loadEncodedBytes(
          cancel: Completer<void>().future,
        );
        final second = provider().loadEncodedBytes(
          cancel: Completer<void>().future,
        );
        final stream = provider().resolve(ImageConfiguration.empty);
        final shown = _waitError(stream);
        await _until(() async => fetched.isNotEmpty);
        respond.complete();
        expect(await first, _pngBytes);
        expect(await second, _pngBytes);
        expect(await shown, isNull);
        expect(fetched, [_relayUrl]);
      } finally {
        await cache.dispose();
      }
    },
  );
}

/// Waits for background work that a fixed delay cannot bound on a slow
/// machine, such as several refreshes running one after another.
Future<void> _until(Future<bool> Function() done) async {
  final deadline = DateTime.now().add(const Duration(seconds: 10));
  while (!await done()) {
    if (DateTime.now().isAfter(deadline)) fail('timed out');
    await Future<void>.delayed(const Duration(milliseconds: 20));
  }
}

/// Lets fire-and-forget disk writes and refreshes finish.
Future<void> _settle() =>
    Future<void>.delayed(const Duration(milliseconds: 200));

Future<Object?> _waitError(ImageStream stream) {
  final done = Completer<Object?>();
  late final ImageStreamListener listener;
  listener = ImageStreamListener(
    (image, sync) {
      image.dispose();
      stream.removeListener(listener);
      if (!done.isCompleted) done.complete(null);
    },
    onError: (error, stack) {
      stream.removeListener(listener);
      if (!done.isCompleted) done.complete(error);
    },
  );
  stream.addListener(listener);
  return done.future;
}

/// [Directory] whose listing waits until [release], to hold a cache's
/// sweep, and so its close.
class _GatedDirectory implements Directory {
  _GatedDirectory(this._inner);

  final Directory _inner;
  final _gate = Completer<void>();

  void release() => _gate.complete();

  @override
  String get path => _inner.path;

  @override
  Future<bool> exists() => _inner.exists();

  @override
  Stream<FileSystemEntity> list({
    bool recursive = false,
    bool followLinks = true,
  }) async* {
    await _gate.future;
    yield* _inner.list(recursive: recursive, followLinks: followLinks);
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
