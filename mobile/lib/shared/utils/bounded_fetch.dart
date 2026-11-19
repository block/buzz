import 'dart:async';
import 'dart:typed_data';

import 'package:http/http.dart' as http;

/// Longest wait for response headers or the next body chunk. A slow link that
/// keeps delivering bytes finishes; one that stops is aborted.
const defaultStallTimeout = Duration(seconds: 20);

/// Longest a fetch may take even while bytes keep arriving, so a server that
/// trickles bytes cannot hold a connection indefinitely.
const defaultMaxFetchDuration = Duration(minutes: 5);

/// A GET response read by [boundedGet]. [bytes] is empty unless the status
/// was 200: an error body is never read.
class BoundedResponse {
  const BoundedResponse(this.statusCode, this.headers, this.bytes);

  final int statusCode;
  final Map<String, String> headers;
  final Uint8List bytes;
}

/// The 200 body of a [boundedGet] exceeded its `maxBytes`.
class ResponseTooLargeException implements Exception {
  const ResponseTooLargeException(this.uri);

  final Uri uri;

  @override
  String toString() => 'ResponseTooLargeException: $uri';
}

/// The caller's `cancel` future completed before [boundedGet] finished.
class FetchCancelledException implements Exception {
  const FetchCancelledException(this.uri);

  final Uri uri;

  @override
  String toString() => 'FetchCancelledException: $uri';
}

/// GETs [uri] without closing [client], aborting the request when no headers
/// or body bytes arrive for [stallTimeout], when it runs past
/// [maxFetchDuration], or when [cancel] completes. A 200 body larger than
/// [maxBytes], declared or streamed, throws [ResponseTooLargeException]; other statuses return
/// without reading the body. A stall or deadline throws the client's abort
/// error ([http.RequestAbortedException]); a cancel throws
/// [FetchCancelledException].
Future<BoundedResponse> boundedGet(
  http.Client client,
  Uri uri, {
  required int maxBytes,
  Map<String, String> headers = const {},
  bool followRedirects = true,
  Duration stallTimeout = defaultStallTimeout,
  Duration maxFetchDuration = defaultMaxFetchDuration,
  Future<void>? cancel,
}) async {
  final stop = Completer<void>();
  var cancelled = false;
  Timer? stall;
  void restartStallTimer() {
    stall?.cancel();
    stall = Timer(stallTimeout, () {
      if (!stop.isCompleted) stop.complete();
    });
  }

  restartStallTimer();
  final deadline = Timer(maxFetchDuration, () {
    if (!stop.isCompleted) stop.complete();
  });
  unawaited(
    cancel?.then((_) {
      if (stop.isCompleted) return;
      cancelled = true;
      stop.complete();
    }),
  );
  StreamIterator<List<int>>? body;
  try {
    final request = http.AbortableRequest('GET', uri, abortTrigger: stop.future)
      ..headers.addAll(headers)
      ..followRedirects = followRedirects;
    final streamed = await client.send(request);
    restartStallTimer();
    if (streamed.statusCode == 200 &&
        (streamed.contentLength ?? 0) > maxBytes) {
      unawaited(streamed.stream.listen(null).cancel());
      throw ResponseTooLargeException(uri);
    }
    final iterator = body = StreamIterator(streamed.stream);
    final bytes = BytesBuilder(copy: false);
    // An error body is never shown, and reading a slow one could hold the
    // caller for the whole fetch cap. Leave it unread; finally cancels it.
    while (streamed.statusCode == 200 && await iterator.moveNext()) {
      restartStallTimer();
      final chunk = iterator.current;
      if (chunk.length > maxBytes - bytes.length) {
        throw ResponseTooLargeException(uri);
      }
      bytes.add(chunk);
    }
    return BoundedResponse(
      streamed.statusCode,
      streamed.headers,
      bytes.takeBytes(),
    );
  } catch (_) {
    if (cancelled) throw FetchCancelledException(uri);
    rethrow;
  } finally {
    stall?.cancel();
    deadline.cancel();
    if (!stop.isCompleted) stop.complete();
    unawaited(body?.cancel());
  }
}
