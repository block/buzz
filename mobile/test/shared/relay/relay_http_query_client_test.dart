import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/relay/relay_http_query_client.dart';
import 'package:buzz/shared/relay/relay_query_response.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

void main() {
  test('the default transport honors http.runWithClient', () async {
    final requests = <http.Request>[];
    final response = await http.runWithClient(
      () => RelayHttpQueryClient().post(
        Uri.parse('https://relay.example/query'),
        headers: const {'Content-Type': 'application/json'},
        body: const [],
        timeout: const Duration(seconds: 5),
      ),
      () => MockClient((request) async {
        requests.add(request);
        return http.Response('[]', 200);
      }),
    );

    expect(response.body, '[]');
    expect(requests.single.headers['Accept-Encoding'], 'gzip');
  });

  test('the default transport leaves a gzip body for the decoder', () async {
    final compressed = gzip.encode(utf8.encode('[]'));
    final server = await HttpServer.bind(InternetAddress.loopbackIPv4, 0);
    addTearDown(() => server.close(force: true));
    server.listen((request) async {
      request.response
        ..headers.set(HttpHeaders.contentEncodingHeader, 'gzip')
        ..headers.contentType = ContentType.json
        ..add(compressed);
      await request.response.close();
    });

    final client = RelayHttpQueryClient();
    addTearDown(client.close);
    final response = await client.post(
      Uri.parse('http://127.0.0.1:${server.port}/query'),
      headers: const {'Content-Type': 'application/json'},
      body: const [],
      timeout: const Duration(seconds: 5),
    );

    expect(response.bodyBytes, compressed);
    expect(
      relayResponseText(response.bodyBytes, response.headers['content-type']),
      '[]',
    );
  });

  test('a timed-out request is aborted, not left running', () async {
    final aborted = Completer<void>();
    final client = RelayHttpQueryClient(
      client: _HangingClient(onAbort: aborted.complete),
    );

    await expectLater(
      client.post(
        Uri.parse('https://relay.example/query'),
        headers: const {},
        body: const [],
        timeout: const Duration(milliseconds: 10),
      ),
      throwsA(isA<TimeoutException>()),
    );
    await aborted.future.timeout(const Duration(seconds: 5));
  });
}

/// Never responds; reports when the request's abort trigger fires.
class _HangingClient extends http.BaseClient {
  _HangingClient({required this.onAbort});

  final void Function() onAbort;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) {
    final trigger = (request as http.Abortable).abortTrigger;
    trigger?.then((_) => onAbort());
    return Completer<http.StreamedResponse>().future;
  }
}
