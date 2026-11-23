import 'dart:async';

import 'package:http/http.dart' as http;

import '../http/app_http_client.dart';

/// One deadline covers headers and the bounded body. Abort only this request,
/// leaving the shared client usable for sign-out and subsequent attempts.
/// [onHeaders] sees the response as soon as its headers arrive, before the
/// body is read, even if reading the body then fails.
Future<http.Response> boundedHttpResponse(
  http.Client client,
  http.Request source, {
  int maxBytes = 65536,
  void Function(http.BaseResponse headers)? onHeaders,
}) async {
  final abort = Completer<void>();
  final request =
      http.AbortableRequest(
          source.method,
          source.url,
          abortTrigger: abort.future,
        )
        ..followRedirects = false
        ..headers.addAll(source.headers)
        ..bodyBytes = source.bodyBytes;
  StreamIterator<List<int>>? body;
  Future<http.Response> read() async {
    final response = await client.send(request);
    try {
      onHeaders?.call(response);
    } catch (_) {
      // Some clients hold the connection until the body is listened to and
      // cancelled; an abort alone leaves it open.
      unawaited(response.stream.listen(null).cancel());
      rethrow;
    }
    final iterator = body = StreamIterator(response.stream);
    if (abort.isCompleted) {
      await iterator.cancel();
      throw http.RequestAbortedException();
    }
    final bytes = <int>[];
    while (await iterator.moveNext()) {
      if (bytes.length + iterator.current.length > maxBytes) {
        throw const FormatException('Response too large');
      }
      bytes.addAll(iterator.current);
    }
    return http.Response.bytes(
      bytes,
      response.statusCode,
      headers: response.headers,
    );
  }

  try {
    return await read().timeout(const Duration(seconds: 30));
  } on TimeoutException {
    noteHttpDeadline(request);
    rethrow;
  } finally {
    abort.complete();
    // Do not let transport cleanup hold the account's serial queue hostage.
    unawaited(body?.cancel());
  }
}
