import 'dart:async';

import 'package:buzz/shared/widgets/ios_media_carousel.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';

void main() {
  testWidgets(
    'native carousel routes callbacks and cancels image work on disposal',
    (tester) async {
      final messenger = tester.binding.defaultBinaryMessenger;
      final channels = <MethodChannel>[];
      MethodChannel? native;
      messenger.setMockMethodCallHandler(SystemChannels.platform_views, (
        call,
      ) async {
        if (call.method == 'create') {
          final id = (call.arguments as Map)['id'];
          native = MethodChannel('${IosMediaCarousel.viewType}/$id');
          channels.add(native!);
          messenger.setMockMethodCallHandler(native!, (_) async => null);
        }
        return null;
      });
      addTearDown(() {
        messenger.setMockMethodCallHandler(SystemChannels.platform_views, null);
        for (final channel in channels) {
          messenger.setMockMethodCallHandler(channel, null);
        }
      });
      final opened = <int>[];
      final pages = <int>[];
      final offsets = <double>[];
      final provider = _PendingImage();
      await tester.pumpWidget(
        MaterialApp(
          home: SizedBox(
            width: 300,
            height: 200,
            child: IosMediaCarousel(
              providers: [provider],
              labels: const ['Photo 1'],
              width: 300,
              gutter: 20,
              gap: 8,
              radius: 16,
              onOpen: opened.add,
              onScroll: offsets.add,
              onPageChanged: pages.add,
            ),
          ),
        ),
      );
      await tester.pump();
      final channel = native!;
      Future<Object?> send(String method, Object argument) async {
        final response = Completer<ByteData?>();
        await messenger.handlePlatformMessage(
          channel.name,
          channel.codec.encodeMethodCall(MethodCall(method, argument)),
          response.complete,
        );
        final bytes = await response.future;
        return bytes == null ? null : channel.codec.decodeEnvelope(bytes);
      }

      await send('open', 0);
      await send('open', 2);
      await send('page', 0);
      await send('scroll', 21.5);
      expect(opened, [0]);
      expect(pages, [0]);
      expect(offsets, [21.5]);
      final pending = send('image', 0);
      await tester.pump();
      await tester.pumpWidget(const SizedBox());
      await tester.pump();
      expect(await pending, isNull);
      expect(tester.takeException(), isNull);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );
}

class _PendingImage extends ImageProvider<_PendingImage> {
  final pending = Completer<ImageInfo>();
  @override
  Future<_PendingImage> obtainKey(ImageConfiguration configuration) =>
      SynchronousFuture(this);
  @override
  ImageStreamCompleter loadImage(
    _PendingImage key,
    ImageDecoderCallback decode,
  ) => OneFrameImageStreamCompleter(pending.future);
}
