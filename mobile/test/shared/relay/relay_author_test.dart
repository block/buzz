import 'package:buzz/shared/relay/relay_author_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

void main() {
  testWidgets(
    'NIP-11 completes from the HTTP stream and trusts self rather than the contact key',
    (tester) async {
      final author = 'a' * 64;
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            relayInformationClientProvider.overrideWith(
              (ref) => MockClient((request) async {
                expect(request.url.toString(), 'https://relay.example');
                expect(request.headers['Accept'], 'application/nostr+json');
                return http.Response(
                  '{"self":"$author","pubkey":"${'b' * 64}"}',
                  200,
                );
              }),
            ),
          ],
          child: MaterialApp(
            home: Consumer(
              builder: (context, ref, _) {
                final value = ref.watch(
                  relayAuthorProvider('wss://relay.example'),
                );
                return Text(value.asData?.value ?? 'Loading');
              },
            ),
          ),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.text(author), findsOneWidget);
    },
  );

  test('a contact key alone cannot establish the community signer', () async {
    final container = ProviderContainer(
      overrides: [
        relayInformationClientProvider.overrideWith(
          (ref) => MockClient(
            (_) async => http.Response('{"pubkey":"${'a' * 64}"}', 200),
          ),
        ),
      ],
    );
    addTearDown(container.dispose);
    final subscription = container.listen(
      relayAuthorProvider('https://relay.example'),
      (_, _) {},
    );
    addTearDown(subscription.close);
    expect(
      await container.read(relayAuthorProvider('https://relay.example').future),
      isNull,
    );
  });

  test('oversized relay information cannot establish authority', () async {
    final container = ProviderContainer(
      overrides: [
        relayInformationClientProvider.overrideWith(
          (ref) => MockClient(
            (_) async => http.Response(' ' * (256 * 1024 + 1), 200),
          ),
        ),
      ],
    );
    addTearDown(container.dispose);
    final subscription = container.listen(
      relayAuthorProvider('https://relay.example'),
      (_, _) {},
    );
    addTearDown(subscription.close);
    await expectLater(
      container.read(relayAuthorProvider('https://relay.example').future),
      throwsFormatException,
    );
  });
}
