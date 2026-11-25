import 'dart:convert';

import 'package:buzz/shared/relay/relay.dart';
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

  test('a failed read is not retried until the relay reconnects', () async {
    var reads = 0;
    final container = ProviderContainer(
      overrides: [
        relaySessionProvider.overrideWith(_Session.new),
        relayInformationClientProvider.overrideWithValue(
          MockClient((request) async {
            reads += 1;
            if (reads == 1) throw http.ClientException('unreachable');
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
    final session = container.read(relaySessionProvider.notifier) as _Session;
    final info = relayInformationProvider(relayUrl);

    await expectLater(read(container), throwsA(isA<http.ClientException>()));
    await Future<void>.delayed(const Duration(seconds: 1));
    expect(container.read(info).hasError, isTrue);
    expect(reads, 1);

    session.status = SessionStatus.reconnecting;
    session.status = SessionStatus.connected;
    expect((await read(container))?.addsAgentsToDm, isTrue);
    expect(reads, 2);
  });
}

class _Session extends RelaySessionNotifier {
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  set status(SessionStatus status) => state = SessionState(status: status);
}
