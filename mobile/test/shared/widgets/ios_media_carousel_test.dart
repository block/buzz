import 'dart:async';
import 'dart:convert';
import 'dart:io';
import 'dart:ui' as ui;

import 'package:buzz/shared/widgets/ios_media_carousel.dart';
import 'package:buzz/shared/relay/media_auth.dart';
import 'package:buzz/shared/relay/media_image.dart';
import 'package:http/http.dart' as http;
import 'package:http/io_client.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  for (final stallBody in [false, true]) {
    testWidgets(
      'stalled ${stallBody ? "bodies" : "headers"} abort before queued photo loads',
      (tester) async {
        final client = (await tester.runAsync(() async => _TrackingClient()))!;
        final server = await tester.runAsync(
          () => HttpServer.bind(InternetAddress.loopbackIPv4, 0),
        );
        addTearDown(() async {
          client.close();
          await server!.close(force: true);
          MediaImageProvider.debugResetCooldowns();
        });
        final base = 'http://127.0.0.1:${server!.port}';
        final auth = MediaGetAuthService(baseUrl: base, nsec: null);
        await tester.runAsync(() async {
          server.listen((request) {
            if (request.uri.path == '/4') {
              request.response.statusCode = 404;
              unawaited(request.response.close());
            } else if (stallBody) {
              request.response.contentLength = 100;
              request.response.add([0]);
              unawaited(request.response.flush());
            }
            // The first four never finish: only client-side abort can release them.
          });
        });
        final harness = await _mount(
          tester,
          List.generate(
            5,
            (index) => MediaImageProvider(
              url: '$base/$index',
              auth: auth,
              client: client,
              requestTimeout: const Duration(seconds: 1),
            ),
          ),
        );
        await tester.runAsync(() async {
          for (var index = 0; index < 4; index++) {
            await harness.send('subscribeImage', {
              'id': '$index',
              'index': index,
            });
            await harness.send('cancelImage', '$index');
          }
          await harness.send('subscribeImage', {'id': 'latest', 'index': 4});
          expect(client.started, 4);
          await harness.waitForFrames(1);
          await client.retired.future.timeout(const Duration(seconds: 5));
        });
        expect(client.started, 5);
        expect(client.peak, 4);
        expect(client.aborted, 4);
        expect(client.active, 0);
        expect(harness.frames, [
          {'id': 'latest', 'bytes': null},
        ]);
        await tester.pumpWidget(const SizedBox());
      },
      variant: TargetPlatformVariant.only(TargetPlatform.iOS),
    );
  }

  testWidgets('native callbacks and disposal release subscribed images', (
    tester,
  ) async {
    final provider = _TestImage();
    final harness = await _mount(tester, [provider]);
    await harness.send('open', 0);
    await harness.send('open', 2);
    await harness.send('page', 0);
    await harness.send('scroll', 21.5);
    expect(harness.opened, [0]);
    expect(harness.pages, [0]);
    expect(harness.offsets, [21.5]);
    await harness.send('subscribeImage', {'id': 'a', 'index': 0});
    expect(provider.completer.listeners, hasLength(1));
    await tester.pumpWidget(const SizedBox());
    expect(provider.completer.listeners, isEmpty);
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets('multi-image native gallery forwards continuing animation frames', (
    tester,
  ) async {
    final animated = _TestImage();
    final still = _TestImage();
    final harness = await _mount(tester, [animated, still]);
    await harness.send('subscribeImage', {'id': 'animated', 'index': 0});
    await harness.send('subscribeImage', {'id': 'still', 'index': 1});
    final decoded = await tester.runAsync(() async {
      // Two 1x1 GIF frames with different palette indices: red, then blue.
      final codec = await ui.instantiateImageCodec(
        base64Decode(
          'R0lGODlhAQABAIAAAP8AAAAA/yH5BAAKAAAALAAAAAABAAEAAAICRAEAIfkEAAoAAAAsAAAAAAEAAQAAAgJMAQA7',
        ),
      );
      try {
        expect(codec.frameCount, 2);
        return [
          (await codec.getNextFrame()).image,
          (await codec.getNextFrame()).image,
        ];
      } finally {
        codec.dispose();
      }
    });
    await tester.runAsync(() async {
      animated.completer.emit(decoded![0]);
      await harness.waitForFrames(1);
      animated.completer.emit(decoded[1]);
      await harness.waitForFrames(2);
    });
    expect(harness.frames.map((f) => f['id']), ['animated', 'animated']);
    expect(
      harness.frames[0]['bytes'],
      isNot(orderedEquals(harness.frames[1]['bytes'] as Uint8List)),
    );
    await harness.send('cancelImage', 'animated');
    expect(animated.completer.listeners, isEmpty);
    final count = harness.frames.length;
    final green = await tester.runAsync(() => _frame(Colors.green));
    animated.completer.emit(green!);
    await tester.pump();
    expect(harness.frames, hasLength(count));
    await tester.pumpWidget(const SizedBox());
    expect(still.completer.listeners, isEmpty);
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));

  testWidgets(
    'rapid reuse bounds unresolved loads and drops cancelled queued requests',
    (tester) async {
      final providers = List.generate(20, (_) => _TestImage());
      final harness = await _mount(tester, providers);
      for (var index = 0; index < 20; index++) {
        await harness.send('subscribeImage', {'id': '$index', 'index': index});
        if (index < 19) await harness.send('cancelImage', '$index');
      }
      expect(providers.where((p) => p.resolutions > 0), hasLength(4));
      expect(
        providers.skip(4).take(15).every((p) => p.completer.listeners.isEmpty),
        isTrue,
      );
      // Retired pending providers retain a bounded completion observer because
      // transport termination must precede releasing that slot.
      final image = await tester.runAsync(() => _frame(Colors.red));
      providers[0].completer.emit(image!);
      await tester.pump();
      expect(providers[0].completer.listeners, isEmpty);
      expect(providers[19].resolutions, 1);
      expect(
        harness.frames,
        isEmpty,
        reason: 'Cancelled loads must never reach UIKit',
      );
      await tester.pumpWidget(const SizedBox());
      expect(providers.every((p) => p.completer.listeners.isEmpty), isTrue);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );
}

Future<ui.Image> _frame(Color color) async {
  final recorder = ui.PictureRecorder();
  Canvas(
    recorder,
  ).drawRect(const Rect.fromLTWH(0, 0, 2, 2), Paint()..color = color);
  final picture = recorder.endRecording();
  try {
    return await picture.toImage(2, 2);
  } finally {
    picture.dispose();
  }
}

Future<_Harness> _mount(
  WidgetTester tester,
  List<ImageProvider<Object>> providers,
) async {
  final harness = _Harness(tester);
  final messenger = tester.binding.defaultBinaryMessenger;
  messenger.setMockMethodCallHandler(SystemChannels.platform_views, (
    call,
  ) async {
    if (call.method == 'create') {
      final id = (call.arguments as Map)['id'];
      harness.channel = MethodChannel('${IosMediaCarousel.viewType}/$id');
      messenger.setMockMethodCallHandler(harness.channel, (call) async {
        if (call.method == 'imageFrame') {
          harness.frames.add(Map<String, Object?>.from(call.arguments as Map));
          harness.changed?.complete();
          harness.changed = null;
        }
        return null;
      });
    }
    return null;
  });
  addTearDown(() {
    messenger.setMockMethodCallHandler(SystemChannels.platform_views, null);
    messenger.setMockMethodCallHandler(harness.channel, null);
  });
  await tester.pumpWidget(
    MaterialApp(
      home: SizedBox(
        width: 300,
        height: 200,
        child: IosMediaCarousel(
          providers: providers,
          labels: List.generate(providers.length, (i) => 'Photo $i'),
          width: 300,
          gutter: 20,
          gap: 4,
          trailingPadding: 20,
          radius: 16,
          onOpen: harness.opened.add,
          onScroll: harness.offsets.add,
          onPageChanged: harness.pages.add,
        ),
      ),
    ),
  );
  await tester.pump();
  return harness;
}

class _Harness {
  _Harness(this.tester);
  final WidgetTester tester;
  late MethodChannel channel;
  final opened = <int>[];
  final pages = <int>[];
  final offsets = <double>[];
  final frames = <Map<String, Object?>>[];
  Completer<void>? changed;
  Future<void> waitForFrames(int count) async {
    while (frames.length < count) {
      changed = Completer<void>();
      await changed!.future.timeout(const Duration(seconds: 5));
    }
  }

  Future<void> send(String method, Object argument) async {
    await tester.binding.defaultBinaryMessenger.handlePlatformMessage(
      channel.name,
      channel.codec.encodeMethodCall(MethodCall(method, argument)),
      (_) {},
    );
  }
}

class _TestImage extends ImageProvider<_TestImage> {
  _TestImage() {
    final handle = completer.keepAlive();
    addTearDown(handle.dispose);
  }
  final completer = _TestCompleter();
  int resolutions = 0;
  @override
  Future<_TestImage> obtainKey(ImageConfiguration configuration) =>
      SynchronousFuture(this);
  @override
  void resolveStreamForKey(
    ImageConfiguration configuration,
    ImageStream stream,
    _TestImage key,
    ImageErrorListener handleError,
  ) {
    resolutions++;
    stream.setCompleter(completer);
  }
}

class _TestCompleter extends ImageStreamCompleter {
  final listeners = <ImageStreamListener>{};
  @override
  void addListener(ImageStreamListener listener) {
    listeners.add(listener);
    super.addListener(listener);
  }

  @override
  void removeListener(ImageStreamListener listener) {
    listeners.remove(listener);
    super.removeListener(listener);
  }

  void emit(ui.Image image) => setImage(ImageInfo(image: image));
}

// Real IO transport: count through body completion, not just response headers.
class _TrackingClient extends http.BaseClient {
  final inner = IOClient(_RealHttpOverrides().createHttpClient(null));
  int active = 0, peak = 0, started = 0, aborted = 0;
  final retired = Completer<void>();
  void recordAbort() {
    aborted++;
    if (aborted == 4) retired.complete();
  }

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    started++;
    active++;
    if (active > peak) peak = active;
    try {
      final response = await inner.send(request);
      return http.StreamedResponse(_body(response.stream), response.statusCode);
    } catch (error) {
      active--;
      if (error is http.RequestAbortedException) recordAbort();
      rethrow;
    }
  }

  Stream<List<int>> _body(Stream<List<int>> stream) async* {
    try {
      await for (final chunk in stream) {
        yield chunk;
      }
    } catch (error) {
      if (error is http.RequestAbortedException) recordAbort();
      rethrow;
    } finally {
      active--;
    }
  }

  @override
  void close() => inner.close();
}

class _RealHttpOverrides extends HttpOverrides {}
