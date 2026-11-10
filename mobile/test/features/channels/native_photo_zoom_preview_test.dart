import 'dart:convert';
import 'dart:async';
import 'dart:ui' as ui;
import 'package:flutter/services.dart';

import 'package:buzz/features/channels/native_photo_zoom_preview.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  TestWidgetsFlutterBinding.ensureInitialized();
  final messenger =
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger;
  final provider = MemoryImage(
    base64Decode(
      'iVBORw0KGgoAAAANSUhEUgAAAAIAAAACCAYAAABytg0kAAAAEElEQVR4nGNgaPj/H4xhDABS0gn5PEa22gAAAABJRU5ErkJggg==',
    ),
  );
  tearDown(() {
    debugDefaultTargetPlatformOverride = null;
    messenger.setMockMethodCallHandler(NativePhotoZoomPreview.channel, null);
  });

  for (final outcome in ['closed', 'reply', 'unavailable']) {
    testWidgets(
      'native photo preview handles $outcome and selected source bounds',
      (tester) async {
        debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
        final key = GlobalKey();
        late BuildContext context;
        await tester.pumpWidget(
          MaterialApp(
            home: Builder(
              builder: (value) {
                context = value;
                return Center(
                  child: SizedBox(key: key, width: 200, height: 200),
                );
              },
            ),
          ),
        );
        Map<dynamic, dynamic>? presented;
        messenger.setMockMethodCallHandler(NativePhotoZoomPreview.channel, (
          call,
        ) async {
          if (call.method == 'isSupported') return true;
          presented = call.arguments as Map;
          return outcome;
        });
        final result = await tester.runAsync(
          () => NativePhotoZoomPreview.open(
            context: context,
            sourceKeys: [key],
            providers: [provider],
            previewProviders: [provider],
            urls: ['https://example.org/photo.jpg'],
            initialIndex: 0,
            onSelect: (_) async {},
            labels: ['Selected photo'],
          ),
        );
        debugDefaultTargetPlatformOverride = null;
        expect(result, outcome == 'unavailable');
        expect(presented?['labels'], ['Selected photo']);
        expect(presented?.containsKey('previews'), isFalse);
        expect(presented?['width'], 200);
        expect(presented?['height'], 200);
        expect(presented?['x'], 400);
        expect(presented?['y'], 300);
        expect(presented?['angle'], 0);
        expect(presented?['bytes'], isA<Uint8List>());
      },
      skip: !NativePhotoZoomPreview.enabled,
    );
  }

  testWidgets(
    'native gallery selects the right URL and enforces action authority',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final keys = [GlobalKey(), GlobalKey()];
      late BuildContext context;
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (value) {
              context = value;
              return Row(
                children: [
                  for (final key in keys)
                    SizedBox(key: key, width: 100, height: 100),
                ],
              );
            },
          ),
        ),
      );
      final pendingPreview = _PendingPreview();
      final selected = <int>[];
      final actions = <String>[];
      var replied = false;
      const codec = StandardMethodCodec();
      Future<Object?> nativeCall(
        String method,
        Map<String, Object> args,
      ) async {
        final reply = Completer<Object?>();
        await messenger.handlePlatformMessage(
          NativePhotoZoomPreview.channel.name,
          codec.encodeMethodCall(MethodCall(method, args)),
          (data) {
            reply.complete(data == null ? null : codec.decodeEnvelope(data));
          },
        );
        return reply.future;
      }

      messenger.setMockMethodCallHandler(NativePhotoZoomPreview.channel, (
        call,
      ) async {
        if (call.method == 'isSupported') return true;
        final args = call.arguments as Map;
        expect(args['initialIndex'], 1);
        expect(pendingPreview.frame.isCompleted, isFalse);
        expect(args.containsKey('previews'), isFalse);
        final codec = await ui.instantiateImageCodec(provider.bytes);
        final frame = await codec.getNextFrame();
        pendingPreview.frame.complete(ImageInfo(image: frame.image));
        codec.dispose();
        expect(args['canReply'], isTrue);
        expect(args['canDelete'], isFalse);
        final payload = <String, Object>{
          'session': args['session'] as int,
          'index': 1,
        };
        expect(await nativeCall('select', payload), isA<Map>());
        expect(await nativeCall('image', payload), isA<Uint8List>());
        expect(
          await nativeCall('preview', {...payload, 'index': 0}),
          isA<Uint8List>(),
        );
        await nativeCall('action', {...payload, 'action': 'copy'});
        await nativeCall('action', {...payload, 'action': 'delete'});
        await nativeCall('action', {...payload, 'index': 99, 'action': 'copy'});
        await nativeCall('action', {
          ...payload,
          'session': -1,
          'action': 'copy',
        });
        return 'reply';
      });
      final fallback = await tester.runAsync(
        () => NativePhotoZoomPreview.open(
          context: context,
          sourceKeys: keys,
          providers: [provider, provider],
          previewProviders: [pendingPreview, provider],
          urls: [
            'https://example.org/first.jpg',
            'https://example.org/second.jpg',
          ],
          labels: ['First', 'Second'],
          initialIndex: 1,
          onSelect: (index) async {
            selected.add(index);
          },
          onReply: () {
            replied = true;
          },
          actions: NativePhotoActions(
            canDelete: false,
            perform: (action, url) async {
              actions.add('$action:$url');
              return null;
            },
          ),
        ),
      );
      debugDefaultTargetPlatformOverride = null;
      expect(fallback, isFalse);
      expect(replied, isTrue);
      expect(selected, [1]);
      expect(actions, ['copy:https://example.org/second.jpg']);
    },
    skip: !NativePhotoZoomPreview.enabled,
  );

  testWidgets(
    'large native gallery loads only nearby requests and abandons work on move and close',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      final key = GlobalKey();
      late BuildContext context;
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (value) {
              context = value;
              return SizedBox(key: key, width: 100, height: 100);
            },
          ),
        ),
      );
      final pending = List.generate(50, (_) => _PendingPreview());
      final requests = <Future<Object?>>[];
      const codec = StandardMethodCodec();
      Future<Object?> nativeCall(String method, int session, int index) async {
        final reply = Completer<Object?>();
        await messenger.handlePlatformMessage(
          NativePhotoZoomPreview.channel.name,
          codec.encodeMethodCall(
            MethodCall(method, {'session': session, 'index': index}),
          ),
          (data) =>
              reply.complete(data == null ? null : codec.decodeEnvelope(data)),
        );
        return reply.future;
      }

      messenger.setMockMethodCallHandler(NativePhotoZoomPreview.channel, (
        call,
      ) async {
        if (call.method == 'isSupported') return true;
        final session = (call.arguments as Map)['session'] as int;
        expect(pending.where((p) => p.started), isEmpty);
        requests.add(nativeCall('preview', session, 1));
        requests.add(nativeCall('image', session, 2));
        expect(await nativeCall('preview', session, 49), isNull);
        await nativeCall('select', session, 25);
        expect(await Future.wait(requests), everyElement(isNull));
        expect(pending[1].abandoned, isTrue);
        expect(pending[2].abandoned, isTrue);
        requests.add(nativeCall('preview', session, 24));
        requests.add(nativeCall('image', session, 26));
        // Drain delivery, without resolving either image, then dismiss.
        await nativeCall('preview', session, 49);
        expect(pending.where((p) => p.started).length, 4);
        return 'closed';
      });
      await tester.runAsync(() async {
        expect(
          await NativePhotoZoomPreview.open(
            context: context,
            sourceKeys: List.filled(50, key),
            providers: pending,
            previewProvider: provider,
            previewProviders: pending,
            urls: List.generate(50, (i) => 'https://example.org/$i.jpg'),
            labels: List.generate(50, (i) => 'Photo $i'),
            initialIndex: 0,
            onSelect: (_) async {},
          ),
          isFalse,
        );
        expect(pending[24].abandoned, isTrue);
        expect(pending[26].abandoned, isTrue);
        expect(await Future.wait(requests), everyElement(isNull));
        // Late provider completion cannot resurrect a closed native session.
        for (final load in pending.where((p) => p.started)) {
          final imageCodec = await ui.instantiateImageCodec(provider.bytes);
          final frame = await imageCodec.getNextFrame();
          load.frame.complete(ImageInfo(image: frame.image));
          imageCodec.dispose();
        }
      });
      await tester.pump();
      expect(tester.takeException(), isNull);
      debugDefaultTargetPlatformOverride = null;
    },
    skip: !NativePhotoZoomPreview.enabled,
  );

  testWidgets('animation-capable galleries retain all frames in Flutter', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    // Two one-pixel frames, red then blue, with a 100ms frame delay.
    final animated = base64Decode(
      'R0lGODlhAQABAIAAAP8AAAAA/yH5BAAKAAAALAAAAAABAAEAAAICRAEAIfkEAAoAAAAsAAAAAAEAAQAAAgJMAQA7',
    );
    final image = MemoryImage(animated);
    late BuildContext context;
    await tester.pumpWidget(
      MaterialApp(
        home: Builder(
          builder: (value) {
            context = value;
            return const SizedBox();
          },
        ),
      ),
    );
    var bridgeCalls = 0;
    messenger.setMockMethodCallHandler(NativePhotoZoomPreview.channel, (
      _,
    ) async {
      bridgeCalls++;
      return false;
    });
    for (final source in [
      ('https://example.org/photo.png', null),
      ('https://example.org/photo.apng', null),
      ('https://example.org/photo.webp', null),
      ('https://example.org/photo.jpg', 'image/gif'),
      ('https://example.org/photo', 'image/webp'),
      ('https://example.org/photo', null),
    ]) {
      expect(
        await NativePhotoZoomPreview.open(
          context: context,
          sourceKeys: [GlobalKey(), GlobalKey()],
          providers: [provider, image],
          urls: ['https://example.org/still.jpg', source.$1],
          mimeTypes: ['image/jpeg', source.$2],
          labels: ['Still', 'Animated'],
          initialIndex: 0,
          onSelect: (_) async {},
        ),
        isTrue,
      );
    }
    expect(
      bridgeCalls,
      0,
      reason: 'Potential animations must bypass the PNG bridge',
    );
    await tester.runAsync(() async {
      final codec = await ui.instantiateImageCodec(animated);
      expect(codec.frameCount, 2);
      final first = await codec.getNextFrame();
      final second = await codec.getNextFrame();
      final firstPixels = await first.image.toByteData();
      final secondPixels = await second.image.toByteData();
      expect(
        firstPixels!.buffer.asUint8List(),
        isNot(secondPixels!.buffer.asUint8List()),
      );
      first.image.dispose();
      second.image.dispose();
      codec.dispose();
    });
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets(
    'unsupported platform preserves existing gallery without decoding',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      late BuildContext context;
      await tester.pumpWidget(
        MaterialApp(
          home: Builder(
            builder: (value) {
              context = value;
              return const SizedBox();
            },
          ),
        ),
      );
      messenger.setMockMethodCallHandler(NativePhotoZoomPreview.channel, (
        call,
      ) async {
        expect(call.method, 'isSupported');
        return false;
      });
      expect(
        await NativePhotoZoomPreview.open(
          context: context,
          sourceKeys: [GlobalKey()],
          providers: [provider],
          urls: ['https://example.org/photo.jpg'],
          initialIndex: 0,
          onSelect: (_) async {},
          labels: ['Photo'],
        ),
        isTrue,
      );
      debugDefaultTargetPlatformOverride = null;
    },
  );
}

class _PendingPreview extends ImageProvider<_PendingPreview> {
  late final frame = Completer<ImageInfo>();
  bool started = false;
  bool abandoned = false;
  @override
  Future<_PendingPreview> obtainKey(ImageConfiguration configuration) =>
      SynchronousFuture(this);
  @override
  ImageStreamCompleter loadImage(
    _PendingPreview key,
    ImageDecoderCallback decode,
  ) {
    started = true;
    return OneFrameImageStreamCompleter(frame.future)
      ..addOnLastListenerRemovedCallback(() {
        abandoned = true;
      });
  }
}
