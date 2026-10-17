// Real-IO transport probes for MediaVideoViewerPage.
//
// These tests exercise the actual IOClient (dart:io) transport path for
// AbortableStreamedRequest.  They are intentionally in a SEPARATE file
// from video_viewer_test.dart because TestWidgetsFlutterBinding.ensureInitialized()
// in the main test file installs a suite-wide HttpClient override that returns
// status 400 for ALL requests — including plain test() calls in the same suite.
// By isolating these here, the real loopback HttpServer can be reached.
//
// Transport probe #1: request sink must be closed before send().
//   A local loopback server reads the full request body before replying.
//   Without the `unawaited(request.sink.close())` fix, IOClient.send() blocks
//   at `stream.pipe(ioRequest)` forever — test times out.
//
// Transport probe #2: abort trigger cancels an in-flight download.
//   The server holds the response open via a teardown-controlled release gate
//   so the abort fires against a genuinely in-flight transfer rather than
//   racing a completed response.  After server confirms request arrival,
//   the abort is triggered; send() must throw the typed
//   RequestAbortedException within a bounded deadline.

import 'dart:async';
import 'dart:io';

import 'package:http/http.dart' as http;
import 'package:http/io_client.dart' show IOClient;
import 'package:flutter_test/flutter_test.dart';

void main() {
  test(
    'Transport: request sink is closed — real IO loopback probe (success)',
    () async {
      final videoBytes = <int>[0, 1, 2, 3];

      // Start a local HTTP server that reads the full request body BEFORE
      // sending the response.  If the sink is not closed, the drain hangs.
      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      addTearDown(() => server.close(force: true));

      server.listen((req) async {
        await req.drain<void>(); // hangs if sink was not closed
        req.response
          ..statusCode = 200
          ..headers.contentType = ContentType('video', 'mp4')
          ..contentLength = videoBytes.length
          ..add(videoBytes);
        await req.response.close();
      });

      final serverUrl =
          'http://${server.address.host}:${server.port}/video.mp4';
      final client = IOClient(
        HttpClient()..idleTimeout = const Duration(milliseconds: 1),
      );
      addTearDown(client.close);

      final requestAbort = Completer<void>();
      final request = http.AbortableStreamedRequest(
        'GET',
        Uri.parse(serverUrl),
        abortTrigger: requestAbort.future,
      );
      // THE FIX: close the sink before send so the pipe completes.
      unawaited(request.sink.close());

      final response = await client
          .send(request)
          .timeout(
            const Duration(seconds: 5),
            onTimeout: () =>
                throw TimeoutException('send() did not complete within 5 s'),
          );

      expect(
        response.statusCode,
        200,
        reason: 'sink closed → server receives full request → replies 200',
      );
      await response.stream.drain<void>().timeout(
        const Duration(seconds: 5),
        onTimeout: () => throw TimeoutException(
          'response drain did not complete within 5 s',
        ),
      );
    },
  );

  test(
    'Transport: abort trigger cancels an in-flight download — real IO loopback',
    () async {
      // The server signals that the request has arrived (so the abort fires
      // AFTER the connection is established, not before).
      final requestArrivedCompleter = Completer<void>();

      // Teardown-controlled gate: the server handler awaits this before
      // calling response.close(), so the response body stays open until the
      // test explicitly releases it (on success) or teardown releases it (on
      // failure).  Without this gate, response.close() would send an empty 200
      // immediately and the abort would race against a completed response —
      // making the probe scheduling-sensitive rather than a controlled
      // in-flight cancellation.
      final releaseResponse = Completer<void>();

      final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
      addTearDown(() => server.close(force: true));
      addTearDown(() {
        if (!releaseResponse.isCompleted) releaseResponse.complete();
      });

      server.listen((req) async {
        await req.drain<void>();
        // Signal that the server has received the request, then hold the
        // response open until the test releases it or teardown fires.
        if (!requestArrivedCompleter.isCompleted) {
          requestArrivedCompleter.complete();
        }
        // Await the teardown-controlled gate before attempting to close.
        // After an abort the client may have already torn down the connection,
        // so close() may throw — caught and discarded here.
        await releaseResponse.future;
        try {
          await req.response.close();
        } catch (_) {
          // Connection may already be torn down by the client abort — expected.
        }
      });

      final serverUrl =
          'http://${server.address.host}:${server.port}/video.mp4';
      final client = IOClient(
        HttpClient()..idleTimeout = const Duration(milliseconds: 1),
      );
      addTearDown(client.close);

      final requestAbort = Completer<void>();
      final request = http.AbortableStreamedRequest(
        'GET',
        Uri.parse(serverUrl),
        abortTrigger: requestAbort.future,
      );
      unawaited(request.sink.close());

      // Start the request, wait for server-arrival confirmation, then abort.
      final sendFuture = client.send(request);
      // Bounded wait: if the server doesn't see the request within 5s, fail.
      await requestArrivedCompleter.future.timeout(
        const Duration(seconds: 5),
        onTimeout: () => throw TimeoutException(
          'Loopback server did not receive request within 5 s',
        ),
      );
      requestAbort.complete();

      // send() must throw RequestAbortedException within a bounded deadline.
      // The typed assertion distinguishes an abort from any other exception.
      await expectLater(
        sendFuture.timeout(const Duration(seconds: 5)),
        throwsA(isA<http.RequestAbortedException>()),
        reason: 'abort must cause send() to throw RequestAbortedException',
      );

      // Release the server handler so it can exit cleanly.  The abort may
      // have already torn down the connection, so close() in the handler may
      // throw and be discarded.  Teardown also releases this gate; this is
      // belt-and-suspenders cleanup for the success path.
      if (!releaseResponse.isCompleted) releaseResponse.complete();
    },
  );
}
