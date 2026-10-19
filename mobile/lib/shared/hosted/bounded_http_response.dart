import 'dart:async';

import 'package:http/http.dart' as http;

/// One deadline covers headers and the bounded body. Abort only this request,
/// leaving the shared client usable for sign-out and subsequent attempts.
Future<http.Response> boundedHttpResponse(
  http.Client client,
  http.Request source,
) async {
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
    final iterator = body = StreamIterator(response.stream);
    if (abort.isCompleted) {
      await iterator.cancel();
      throw http.RequestAbortedException();
    }
    final bytes = <int>[];
    while (await iterator.moveNext()) {
      if (bytes.length + iterator.current.length > 65536) {
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
  } finally {
    abort.complete();
    // Do not let transport cleanup hold the account's serial queue hostage.
    unawaited(body?.cancel());
  }
}
