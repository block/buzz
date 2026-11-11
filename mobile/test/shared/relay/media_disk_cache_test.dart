import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:typed_data';

import 'package:buzz/shared/relay/media_auth.dart';
import 'package:buzz/shared/relay/media_disk_cache.dart';
import 'package:buzz/shared/relay/media_image.dart';
import 'package:flutter/painting.dart';
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
const _hugeUrl =
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
    // While set, external images answer once it completes, with an image
    // too large to save.
    Completer<void>? replaceExternal;
    // While set, external images answer with a body that never decodes.
    var malformedExternal = false;
    // Trailing bytes after IEND still decode, so this is a 5 MiB+ image.
    final hugeBytes = [..._pngBytes, ...Uint8List(MediaDiskCache.maxFileBytes)];
    final client = http_testing.MockClient((request) async {
      final url = request.url.toString();
      fetched.add(url);
      if (!online) throw const SocketException('offline');
      if (url == _hugeUrl) return http.Response.bytes(hugeBytes, 200);
      if (malformedExternal && _externalUrls.contains(url)) {
        return http.Response('not an image', 200);
      }
      if (replaceExternal != null && _externalUrls.contains(url)) {
        await replaceExternal.future;
        return http.Response.bytes(hugeBytes, 200);
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

    const saved = [_relayUrl, _thumbnailUrl, ..._externalUrls];
    const urls = [...saved, _hugeUrl];

    // First launch downloads everything and saves what fits.
    await launch((cache) async {
      for (final url in urls) {
        expect(await show(cache, url), isNull, reason: url);
      }
      expect(await show(cache, _corruptUrl), isNotNull);
      expect(await show(cache, _truncatedUrl), isNotNull);
      await _settle();
    });
    expect(fetched, [...urls, _corruptUrl, _truncatedUrl]);

    // Offline relaunch: the original and its thumbnail are separate entries,
    // both shown from disk. The oversized image and the bodies that failed
    // to decode were never saved.
    fetched.clear();
    online = false;
    await launch((cache) async {
      for (final url in saved) {
        expect(await show(cache, url), isNull, reason: url);
      }
      expect(await show(cache, _hugeUrl), isNotNull);
      expect(await show(cache, _corruptUrl), isNotNull);
      expect(await show(cache, _truncatedUrl), isNotNull);
    });
    expect(fetched, [_hugeUrl, _corruptUrl, _truncatedUrl]);

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
      // The third waits for a slot. The replacements are too large to save,
      // so the old copies go rather than showing in their place.
      replacement.complete();
      await _until(() async {
        for (final url in _externalUrls) {
          if (await cache.read(url) != null) return false;
        }
        return true;
      });
      expect(fetched, _externalUrls);
    });
    replaceExternal = null;

    fetched.clear();
    online = false;
    await launch((cache) async {
      for (final url in _externalUrls) {
        expect(await show(cache, url), isNotNull, reason: url);
      }
    });
    expect(fetched, _externalUrls);

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

    // A replaced account's directory goes, including writes that land after
    // its delete; removing the community deletes its directory.
    expect(await directory.exists(), isTrue);
    await MediaDiskCache.removeOrphans(root, () => {'community-1': 'signer-2'});
    expect(await directory.exists(), isFalse);
    await directory.create(recursive: true);
    await MediaDiskCache.removeOrphans(root, () => {'community-1': 'signer-1'});
    expect(await directory.exists(), isTrue);
    await MediaDiskCache.removeOrphans(root, () => {'community-2': 'signer-1'});
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
