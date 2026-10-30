import 'dart:convert';

import 'package:buzz/shared/community/community_icon_provider.dart';
import 'package:buzz/shared/community/thread_policy_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';

class _ConnectedSession extends RelaySessionNotifier {
  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);
}

void main() {
  const relay = 'wss://relay.example.com';

  Future<ThreadPolicy> policyFor(http.Response response) async {
    final container = ProviderContainer(
      retry: (_, _) => null,
      overrides: [
        relaySessionProvider.overrideWith(_ConnectedSession.new),
        communityIconHttpClientProvider.overrideWithValue(
          MockClient((request) async {
            expect(request.url.toString(), 'https://relay.example.com/');
            expect(request.headers['Accept'], 'application/nostr+json');
            return response;
          }),
        ),
      ],
    );
    addTearDown(container.dispose);
    return container.read(communityThreadPolicyProvider(relay).future);
  }

  test('only the exact NIP-11 value "flat" selects flat', () async {
    final cases = <Object?, ThreadPolicy>{
      'flat': ThreadPolicy.flat,
      'nested': ThreadPolicy.nested,
      'FLAT': ThreadPolicy.nested,
      'slack': ThreadPolicy.nested,
      true: ThreadPolicy.nested,
      null: ThreadPolicy.nested,
    };
    for (final MapEntry(key: value, value: expected) in cases.entries) {
      final document = value == null ? {} : {'thread_policy': value};
      expect(
        await policyFor(http.Response(jsonEncode(document), 200)),
        expected,
        reason: '$value',
      );
    }
  });

  test('an unreachable relay is an error, not nested', () async {
    await expectLater(
      policyFor(http.Response('unavailable', 503)),
      throwsException,
    );
  });
}
