import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/hosted/nip_fi_discovery.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

void main() {
  test(
    'NIP-FI requirement follows discovery, and malformed documents fail',
    () {
      final cases = <(Object?, bool?)>[
        // Shadow mode advertises support without requiring evidence.
        (
          {
            'limitation': {'federated_identity': true},
            'requires_nip_fi': false,
          },
          false,
        ),
        (
          {
            'limitation': {'federated_identity': true},
            'requires_nip_fi': true,
          },
          true,
        ),
        // Without the host flag, a relay requires what it supports.
        (
          {
            'limitation': {'federated_identity': true},
          },
          true,
        ),
        (
          {
            'limitation': {'federated_identity': false},
          },
          false,
        ),
        ({'limitation': <String, dynamic>{}}, false),
        ({'name': 'relay'}, false),
        ({'requires_nip_fi': true}, true),
        // Null marks a malformed document.
        (
          {
            'limitation': {'federated_identity': 'yes'},
          },
          null,
        ),
        ({'limitation': true}, null),
        ({'requires_nip_fi': 'true'}, null),
        ([], null),
        ('relay', null),
      ];
      for (final (document, required) in cases) {
        if (required == null) {
          expect(
            () => parseNipFiRequirement(document),
            throwsFormatException,
            reason: '$document',
          );
        } else {
          expect(
            parseNipFiRequirement(document),
            required,
            reason: '$document',
          );
        }
      }
    },
  );

  test(
    'discovery reads the root document, bounded and uncredentialed',
    () async {
      final requests = <http.BaseRequest>[];
      Future<bool> discover(
        http.StreamedResponse Function() respond, {
        Duration deadline = const Duration(seconds: 10),
      }) => fetchNipFiRequirement(
        MockClient.streaming((request, _) async {
          requests.add(request);
          return respond();
        }),
        Uri.parse('https://one.example.org/ignored?x=1'),
        deadline: deadline,
      );
      http.StreamedResponse document(Object body, {int status = 200}) =>
          http.StreamedResponse(
            Stream.value(utf8.encode(jsonEncode(body))),
            status,
          );

      expect(
        await discover(
          () => document({
            'limitation': {'federated_identity': true},
            'requires_nip_fi': false,
          }),
        ),
        isFalse,
      );
      expect(requests.single.url.toString(), 'https://one.example.org/');
      expect(requests.single.headers, {'Accept': 'application/nostr+json'});
      expect(requests.single.followRedirects, isFalse);

      await expectLater(
        discover(() => document({'error': 'unknown'}, status: 404)),
        throwsA(isA<http.ClientException>()),
      );
      await expectLater(
        discover(
          () => http.StreamedResponse(
            Stream.fromIterable(List.filled(300, List.filled(1024, 0x20))),
            200,
          ),
        ),
        throwsFormatException,
      );
      final stalled = StreamController<List<int>>();
      addTearDown(stalled.close);
      await expectLater(
        discover(
          () => http.StreamedResponse(stalled.stream, 200),
          deadline: const Duration(milliseconds: 50),
        ),
        throwsA(isA<TimeoutException>()),
      );
    },
  );
}
