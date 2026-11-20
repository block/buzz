import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/verified_profiles/verified_profile.dart';
import 'package:buzz/shared/verified_profiles/verified_profile_discovery_fetch.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

void main() {
  final uri = Uri.parse('https://relay.example.com');

  test('a body that keeps trickling still hits the overall deadline', () async {
    var cancelled = false;
    final client = MockClient.streaming((request, _) async {
      late final StreamController<List<int>> body;
      body = StreamController<List<int>>(
        onListen: () {
          // Each chunk arrives well inside any per-chunk timeout.
          final timer = Timer.periodic(
            const Duration(milliseconds: 20),
            (_) => body.add(utf8.encode(' ')),
          );
          body.onCancel = () {
            cancelled = true;
            timer.cancel();
          };
        },
      );
      return http.StreamedResponse(body.stream, 200);
    });
    await expectLater(
      fetchVerifiedProfileDiscovery(
        client,
        uri,
        deadline: const Duration(milliseconds: 200),
      ),
      throwsA(isA<TimeoutException>()),
    );
    await pumpEventQueue();
    expect(cancelled, isTrue);
  });

  test('a complete document is parsed', () async {
    final self = 'c' * 64;
    final client = MockClient(
      (request) async => http.Response(
        jsonEncode({
          'self': self,
          'federated_identity': {
            'public_profile': {'kind': 30382, 'multiple_issuers': false},
          },
        }),
        200,
      ),
    );
    final result = await fetchVerifiedProfileDiscovery(client, uri);
    expect(result, isA<VerifiedProfilesEnabled>());
    expect((result as VerifiedProfilesEnabled).authority.pubkey, self);
  });
}
