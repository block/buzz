part of 'message_content_test.dart';

void carouselPreviewTests() {
  testWidgets(
    'iOS prepares five thumbnails before scrolling with two preload requests',
    (tester) async {
      final started = <String>[];

      final responses = <String, Completer<http.Response>>{};
      final urls = List.generate(
        10,
        (i) => 'https://example.com/preload-$i.png',
      );
      final messenger = tester.binding.defaultBinaryMessenger;
      const connectivity = MethodChannel(
        'dev.fluttercommunity.plus/connectivity_status',
      );
      messenger.setMockMethodCallHandler(connectivity, (_) async => null);
      addTearDown(() => messenger.setMockMethodCallHandler(connectivity, null));
      final channels = <MethodChannel>[];
      messenger.setMockMethodCallHandler(SystemChannels.platform_views, (
        call,
      ) async {
        if (call.method == 'create') {
          final id = (call.arguments as Map)['id'];
          final channel = MethodChannel('buzz/ios_media_carousel/$id');
          channels.add(channel);
          messenger.setMockMethodCallHandler(channel, (_) async => null);
        }
        return null;
      });
      final client = MockClient((request) {
        final url = request.url.toString();
        started.add(url);
        final response = responses[url] = Completer<http.Response>();

        return response.future;
      });
      Future<void> waitForCount(int count) async {
        for (
          var attempt = 0;
          attempt < 100 && started.length < count;
          attempt++
        ) {
          await tester.pump();
          await tester.runAsync(
            () => Future<void>.delayed(const Duration(milliseconds: 10)),
          );
        }
        expect(started.length, count);
      }

      final png = base64Decode(
        'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mP8/x8AAwMCAO+aL1sAAAAASUVORK5CYII=',
      );
      void release(String url) {
        final response = responses[url]!;
        if (!response.isCompleted) {
          response.complete(http.Response.bytes(png, 200));
        }
      }

      addTearDown(() async {
        for (final response in responses.values) {
          if (!response.isCompleted) response.complete(http.Response('', 404));
        }
        messenger.setMockMethodCallHandler(SystemChannels.platform_views, null);
        for (final channel in channels) {
          messenger.setMockMethodCallHandler(channel, null);
        }

        client.close();
      });
      await tester.pumpWidget(
        _testable(
          MessageContent(
            content: urls.map((url) => '![image]($url)').join('\n'),
          ),
          overrides: [
            mediaGetAuthServiceProvider.overrideWithValue(
              MediaGetAuthService(baseUrl: 'https://example.com', nsec: null),
            ),
            mediaHttpClientProvider.overrideWithValue(client),
          ],
        ),
      );
      await tester.pump();
      {
        await waitForCount(2);
        expect(started, hasLength(2));
        for (var i = 0; i < 3; i++) {
          release(started[i]);
          await waitForCount(i + 3);
          expect(started, hasLength(i + 3));
        }
        release(started[3]);
        release(started[4]);
      }
      await tester.pumpAndSettle();
      expect(started.toSet(), urls.take(5).toSet());
      // No native subscribeImage or page callback was sent by this harness.
      await tester.pumpWidget(const SizedBox());
      expect(tester.takeException(), isNull);
    },
    variant: TargetPlatformVariant.only(TargetPlatform.iOS),
  );

  testWidgets('outgoing gallery starts at the right and mirrors its spacing', (
    tester,
  ) async {
    final messenger = tester.binding.defaultBinaryMessenger;
    final channels = <MethodChannel>[];
    final configs = <Map<Object?, Object?>>[];
    messenger.setMockMethodCallHandler(SystemChannels.platform_views, (
      call,
    ) async {
      if (call.method == 'create') {
        final id = (call.arguments as Map)['id'];
        final channel = MethodChannel('buzz/ios_media_carousel/$id');
        channels.add(channel);
        messenger.setMockMethodCallHandler(channel, (call) async {
          if (call.method == 'configure') {
            configs.add(Map<Object?, Object?>.from(call.arguments as Map));
          }
          return null;
        });
      }
      return null;
    });
    addTearDown(() {
      messenger.setMockMethodCallHandler(SystemChannels.platform_views, null);
      for (final channel in channels) {
        messenger.setMockMethodCallHandler(channel, null);
      }
    });
    final counts = <Rect>[];
    final widths = <double>[];
    for (final own in [false, true]) {
      await tester.pumpWidget(
        _testable(
          Padding(
            padding: const EdgeInsets.symmetric(horizontal: 50),
            child: MessageContent(
              key: ValueKey(own),
              ownMessage: own,
              bubble: true,
              mediaCarouselTrailingOverflow: Grid.gutter,
              content:
                  '![one](https://example.com/mirror-one.png)\n![two](https://example.com/mirror-two.png)',
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      counts.add(
        tester.getRect(
          find.byKey(const ValueKey('message-media-carousel-count')),
        ),
      );
      widths.add(
        tester
            .getSize(find.byKey(const ValueKey('message-media-carousel')))
            .width,
      );
      expect(configs.last['rtl'], own);
    }
    expect(widths[0], widths[1]);
    expect(counts[0].left, closeTo(800 - counts[1].right, 0.1));
    await tester.pumpWidget(const SizedBox());
    expect(tester.takeException(), isNull);
  }, variant: TargetPlatformVariant.only(TargetPlatform.iOS));
}
