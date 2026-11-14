import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/http/app_http_client.dart';
import 'package:buzz/shared/utils/bounded_fetch.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

void main() {
  test('without a native session, clients come from http.Client()', () {
    final mock = MockClient((_) async => http.Response('', 200));
    for (final traffic in HttpTraffic.values) {
      expect(
        http.runWithClient(() => newAppHttpClient(traffic), () => mock),
        same(mock),
      );
    }
  });

  test('closing a shared-session handle leaves the session usable', () async {
    var sessionClosed = false;
    final session = _ClosingMock(
      (_) async => http.Response('ok', 200),
      onClose: () => sessionClosed = true,
    );
    final first = SharedSessionClient(session);
    final second = SharedSessionClient(session);

    first.close();

    expect(sessionClosed, isFalse);
    expect((await second.get(Uri.parse('https://relay.example/'))).body, 'ok');
  });

  test('closing a handle cancels its own in-flight requests only', () async {
    final session = _AbortRecorder();
    final first = SharedSessionClient(session);
    final second = SharedSessionClient(session);
    unawaited(
      first
          .get(Uri.parse('https://relay.example/a'))
          .then<void>((_) {}, onError: (_) {}),
    );
    unawaited(
      second
          .get(Uri.parse('https://relay.example/b'))
          .then<void>((_) {}, onError: (_) {}),
    );
    await pumpEventQueue();

    first.close();
    await pumpEventQueue();

    expect(session.aborted, [Uri.parse('https://relay.example/a')]);
    await expectLater(
      first.get(Uri.parse('https://relay.example/c')),
      throwsA(isA<http.ClientException>()),
    );
  });

  test('finished requests hold no abort trigger until close', () async {
    var fired = 0;
    var sessionBodyCancelled = false;
    final client = SharedSessionClient(
      MockClient.streaming((request, _) async {
        (request as http.Abortable).abortTrigger?.then((_) => fired++);
        if (request.url.path == '/video') {
          final body = StreamController<List<int>>(
            onCancel: () => sessionBodyCancelled = true,
          );
          return http.StreamedResponse(body.stream, 500);
        }
        final status = request.url.path == '/missing' ? 404 : 200;
        return http.StreamedResponse(Stream.value([1]), status);
      }),
    );
    for (var i = 0; i < 3; i++) {
      await client.get(Uri.parse('https://relay.example/$i'));
    }
    // A non-200 body is never read, and boundedGet fires its own trigger.
    final missing = await boundedGet(
      client,
      Uri.parse('https://relay.example/missing'),
      maxBytes: 10,
    );
    expect(missing.statusCode, 404);
    await pumpEventQueue();
    // A body cancelled before its first event still cancels the session's
    // download.
    final video = await client.send(
      http.Request('GET', Uri.parse('https://relay.example/video')),
    );
    await video.stream.listen(null).cancel();
    expect(sessionBodyCancelled, isTrue);
    final ownAborts = fired;

    client.close();
    await pumpEventQueue();

    expect(fired, ownAborts);
  });

  test('a request\'s own abort trigger still cancels it', () async {
    final session = _AbortRecorder();
    final client = SharedSessionClient(session);
    final abort = Completer<void>();
    unawaited(
      client
          .send(
            http.AbortableRequest(
              'GET',
              Uri.parse('https://relay.example/a'),
              abortTrigger: abort.future,
            ),
          )
          .then<void>((_) {}, onError: (_) {}),
    );
    await pumpEventQueue();

    abort.complete();
    await pumpEventQueue();

    expect(session.aborted, [Uri.parse('https://relay.example/a')]);
  });

  test('requests reach the session with their headers and body', () async {
    final received =
        <({String method, Map<String, String> headers, String body})>[];
    final client = SharedSessionClient(
      MockClient.streaming((request, body) async {
        received.add((
          method: request.method,
          headers: request.headers,
          body: utf8.decode(await body.toBytes()),
        ));
        return http.StreamedResponse(const Stream.empty(), 200);
      }),
    );

    await client.post(
      Uri.parse('https://relay.example/query'),
      headers: {'authorization': 'Nostr x'},
      body: '[]',
    );
    final upload = http.MultipartRequest(
      'PUT',
      Uri.parse('https://relay.example/upload'),
    )..fields['name'] = 'value';
    await client.send(upload);

    expect(received[0].method, 'POST');
    expect(received[0].headers['authorization'], 'Nostr x');
    expect(received[0].body, '[]');
    expect(received[1].method, 'PUT');
    expect(
      received[1].headers['content-type'],
      startsWith('multipart/form-data'),
    );
    expect(received[1].body, contains('value'));
  });

  test(
    'a streamed body is read only as fast as the session reads it',
    () async {
      var produced = 0;
      Stream<List<int>> chunks() async* {
        for (var i = 0; i < 100; i++) {
          produced++;
          yield [i];
        }
      }

      final request = http.StreamedRequest(
        'PUT',
        Uri.parse('https://relay.example/upload'),
      )..contentLength = 100;
      unawaited(
        request.sink.addStream(chunks()).whenComplete(request.sink.close),
      );
      final client = SharedSessionClient(_AbortRecorder());
      unawaited(client.send(request).then<void>((_) {}, onError: (_) {}));
      await pumpEventQueue();

      // Upload progress counts bytes as they are produced, so a copy that
      // drained the body ahead of the transport would report it sent.
      expect(produced, lessThan(10));
      client.close();
    },
  );

  test('cleartext requests use the handle\'s own client', () async {
    final sessionUrls = <Uri>[];
    final cleartextUrls = <Uri>[];
    var cleartextClosed = false;
    final client = SharedSessionClient(
      MockClient((request) async {
        sessionUrls.add(request.url);
        return http.Response('', 200);
      }),
      cleartext: () => _ClosingMock((request) async {
        cleartextUrls.add(request.url);
        return http.Response('', 200);
      }, onClose: () => cleartextClosed = true),
    );

    await client.get(Uri.parse('https://relay.example/a'));
    await client.get(Uri.parse('http://icons.example/b'));
    client.close();

    expect(sessionUrls, [Uri.parse('https://relay.example/a')]);
    expect(cleartextUrls, [Uri.parse('http://icons.example/b')]);
    expect(cleartextClosed, isTrue);
  });

  test(
    'streamed uploads keep dart:io when the session buffers bodies',
    () async {
      final sessionRequests = <String>[];
      final fallbackRequests = <String>[];
      final client = SharedSessionClient(
        MockClient.streaming((request, body) async {
          await body.drain<void>();
          sessionRequests.add('${request.method} ${request.url.path}');
          return http.StreamedResponse(const Stream.empty(), 200);
        }),
        cleartext: () => MockClient.streaming((request, body) async {
          await body.drain<void>();
          fallbackRequests.add('${request.method} ${request.url.path}');
          return http.StreamedResponse(const Stream.empty(), 200);
        }),
        streamsUploads: false,
      );

      final upload = http.StreamedRequest(
        'PUT',
        Uri.parse('https://media.example/upload'),
      );
      upload.sink
        ..add([1, 2, 3])
        ..close();
      await client.send(upload);
      final download = http.StreamedRequest(
        'GET',
        Uri.parse('https://media.example/video'),
      );
      unawaited(download.sink.close());
      await client.send(download);
      await client.post(Uri.parse('https://relay.example/query'), body: '{}');
      client.close();

      expect(fallbackRequests, ['PUT /upload']);
      expect(sessionRequests, ['GET /video', 'POST /query']);
    },
  );

  test('URLSession error codes map to user-facing failure kinds', () {
    expect(urlErrorFailure(-1009), TransportFailure.unreachable);
    expect(urlErrorFailure(-1004), TransportFailure.unreachable);
    expect(urlErrorFailure(-1001), TransportFailure.timedOut);
    expect(urlErrorFailure(-1200), TransportFailure.insecure);
    expect(urlErrorFailure(-1202), TransportFailure.insecure);
    expect(urlErrorFailure(-999), isNull);
    expect(transportFailure(http.ClientException('offline')), isNull);
  });

  test('Cronet error codes map to user-facing failure kinds', () {
    expect(cronetErrorFailure(4, -7), TransportFailure.timedOut);
    expect(cronetErrorFailure(6, -118), TransportFailure.timedOut);
    expect(cronetErrorFailure(1, -105), TransportFailure.unreachable);
    expect(cronetErrorFailure(2, -106), TransportFailure.unreachable);
    expect(cronetErrorFailure(7, -102), TransportFailure.unreachable);
    expect(cronetErrorFailure(11, -202), TransportFailure.insecure);
    expect(cronetErrorFailure(11, -107), TransportFailure.insecure);
    expect(cronetErrorFailure(11, -324), isNull);
    expect(cronetErrorFailure(10, -356), isNull);
  });
}

class _ClosingMock extends MockClient {
  _ClosingMock(super.fn, {required this.onClose});

  final void Function() onClose;

  @override
  void close() {
    onClose();
    super.close();
  }
}

/// Never responds; records which requests were aborted.
class _AbortRecorder extends http.BaseClient {
  final aborted = <Uri>[];

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) {
    final response = Completer<http.StreamedResponse>();
    (request as http.Abortable).abortTrigger?.then((_) {
      aborted.add(request.url);
      response.completeError(http.RequestAbortedException(request.url));
    });
    return response.future;
  }
}
