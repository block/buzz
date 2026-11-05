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
    // Trailing bytes after IEND still decode, so this is a 5 MiB+ image.
    final hugeBytes = [..._pngBytes, ...Uint8List(MediaDiskCache.maxFileBytes)];
    final client = http_testing.MockClient((request) async {
      final url = request.url.toString();
      fetched.add(url);
      if (!online) throw const SocketException('offline');
      if (url == _hugeUrl) return http.Response.bytes(hugeBytes, 200);
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
      final cache = MediaDiskCache.open(directory, now: () => now);
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
      // An image shown before the cache is ready restarts once it is.
      MediaImageProvider provider(MediaDiskCache? diskCache) =>
          MediaImageProvider(
            url: _relayUrl,
            auth: auth,
            client: client,
            diskCache: diskCache,
          );
      expect(provider(null), isNot(provider(cache)));
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
      // The replacements are too large to save, so the old copies go
      // rather than showing in their place.
      replacement.complete();
      await _settle();
    });
    replaceExternal = null;

    fetched.clear();
    online = false;
    await launch((cache) async {
      for (final url in _externalUrls.take(2)) {
        expect(await show(cache, url), isNotNull, reason: url);
      }
      expect(await show(cache, _externalUrls.last), isNull);
      await _settle();
    });
    // The third, skipped before, retries its refresh.
    expect(fetched, _externalUrls);

    // Removing the community deletes its directory.
    expect(await directory.exists(), isTrue);
    await MediaDiskCache.removeOrphans(root, {'community-2'});
    expect(
      await MediaDiskCache.communityDirectory(root, 'community-1').exists(),
      isFalse,
    );
  });

  test(
    'evicts least recently used files past the byte or file budget',
    () async {
      final directory = await Directory(p.join(root.path, 'media')).create();
      final index = MediaCacheIndex(
        File(p.join(directory.path, 'index.json')),
        directory,
        10,
      );
      await index.open();
      final start = DateTime.utc(2026, 11, 5);
      // (name, size, last used minutes after start)
      const files = [('old', 4, 0), ('newest', 4, 3), ('middle', 4, 2)];
      for (final (name, size, _) in files) {
        await File(p.join(directory.path, name)).writeAsBytes(Uint8List(size));
        await index.updateOrInsert(
          CacheObject(
            name,
            relativePath: name,
            validTill: start.add(const Duration(days: 1)),
          ),
        );
      }
      // updateOrInsert stamps touched as now; restore the scripted order.
      for (final (name, _, minutes) in files) {
        final object = (await index.get(name))!;
        await index.update(
          CacheObject.fromMap({
            ...object.toMap(setTouchedToNow: false),
            CacheObject.columnTouched: start
                .add(Duration(minutes: minutes))
                .millisecondsSinceEpoch,
          }),
          setTouchedToNow: false,
        );
      }

      // (file capacity, evicted): 12 bytes are over the 10-byte budget.
      const cases = [
        (100, ['old']),
        (1, ['middle', 'old']),
      ];
      for (final (capacity, evicted) in cases) {
        final over = await index.getObjectsOverCapacity(capacity);
        expect(over.map((object) => object.key), evicted, reason: '$capacity');
      }
      await index.close();
    },
  );
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
