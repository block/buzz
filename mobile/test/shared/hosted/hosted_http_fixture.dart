import 'dart:async';

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
