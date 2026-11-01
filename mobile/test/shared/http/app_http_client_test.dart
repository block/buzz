import 'package:buzz/shared/http/app_http_client.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

void main() {
  test('off iOS, clients come from http.Client()', () {
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
