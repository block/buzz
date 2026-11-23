import 'dart:async';
import 'dart:io';

import 'package:flutter_test/flutter_test.dart';

import 'package:http/http.dart' as http;

/// A transport that keeps every idle interval short but exceeds total duration.
/// It deliberately ignores abort so the caller must also cancel its reader.
class TrickleClient extends http.BaseClient {
  TrickleClient(this.respond);
  final http.Response Function(http.BaseRequest) respond;
  bool slow = true;
  bool aborted = false;
  bool cancelled = false;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async {
    await request.finalize().drain<void>();
    if (!slow) {
      final response = respond(request);
      return http.StreamedResponse(
        Stream.value(response.bodyBytes),
        response.statusCode,
      );
    }
    if (request is http.Abortable) {
      unawaited(request.abortTrigger?.then((_) => aborted = true));
    }
    await Future<void>.delayed(const Duration(seconds: 20));
    Timer? timer;
    final stream = StreamController<List<int>>(
      onCancel: () {
        cancelled = true;
        timer?.cancel();
      },
    );
    timer = Timer.periodic(const Duration(seconds: 5), (_) => stream.add([32]));
    return http.StreamedResponse(stream.stream, 200);
  }
}

/// Sends every connection to the loopback relay on [port], whatever address
/// it was made for.
class LoopbackHttpOverrides extends HttpOverrides {
  LoopbackHttpOverrides(this.port);

  final int port;

  @override
  HttpClient createHttpClient(SecurityContext? context) =>
      _LoopbackClient(super.createHttpClient(context), port);
}

class _LoopbackClient extends Fake implements HttpClient {
  _LoopbackClient(this._inner, this._port);

  final HttpClient _inner;
  final int _port;

  @override
  String? get userAgent => _inner.userAgent;

  @override
  set userAgent(String? value) => _inner.userAgent = value;

  @override
  Future<HttpClientRequest> openUrl(String method, Uri url) => _inner.openUrl(
    method,
    url.replace(scheme: 'http', host: '127.0.0.1', port: _port),
  );

  @override
  void close({bool force = false}) => _inner.close(force: force);
}
