import 'dart:convert';

import 'package:buzz/shared/relay/relay_author_provider.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

void main() {
  const relayUrl = 'wss://relay.example';
  final author = 'a' * 64;

  Future<RelayInformation?> read(ProviderContainer container) async {
    final subscription = container.listen(
      relayInformationProvider(relayUrl),
      (_, _) {},
    );
    try {
      return await container.read(relayInformationProvider(relayUrl).future);
    } finally {
      subscription.close();
    }
  }

  test('reads the relay signer and whether it adds agents to a DM', () async {
    for (final (info, addsAgents) in [
      (
        {
          'self': author,
          'supported_extensions': ['buzz-dm-agents'],
        },
        true,
      ),
      (
        {
          'self': author,
          'supported_extensions': ['nip-cl'],
        },
        false,
      ),
      ({'self': author, 'supported_extensions': 'buzz-dm-agents'}, false),
      ({'self': author}, false),
    ]) {
      final container = ProviderContainer(
        overrides: [
          relayInformationClientProvider.overrideWithValue(
            MockClient((request) async {
              expect(request.url.toString(), 'https://relay.example');
              return http.Response(jsonEncode(info), 200);
            }),
          ),
        ],
      );
      addTearDown(container.dispose);
      final loaded = await read(container);
      expect(loaded?.author, author);
      expect(loaded?.addsAgentsToDm, addsAgents);
    }
  });

  test('keeps a loaded document, and reads again after a failure', () async {
    var reads = 0;
    final container = ProviderContainer(
      overrides: [
        relayInformationClientProvider.overrideWithValue(
          MockClient((request) async {
            reads += 1;
            if (reads == 1) return http.Response('', 503);
            return http.Response(
              jsonEncode({
                'supported_extensions': ['buzz-dm-agents'],
              }),
              200,
            );
          }),
        ),
      ],
    );
    addTearDown(container.dispose);

    expect(await read(container), isNull);
    await Future<void>.delayed(Duration.zero);
    expect((await read(container))?.addsAgentsToDm, isTrue);
    await Future<void>.delayed(Duration.zero);
    // Nobody watches it now, and the loaded document is still there.
    expect(
      container.read(relayInformationProvider(relayUrl)).asData?.value,
      isNotNull,
    );
    expect(reads, 2);
  });
}
