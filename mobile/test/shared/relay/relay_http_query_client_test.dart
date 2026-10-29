import 'package:buzz/shared/relay/relay_http_query_client.dart';
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
}
