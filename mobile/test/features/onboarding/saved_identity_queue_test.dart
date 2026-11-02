import 'dart:async';
import 'dart:convert';

import 'package:buzz/features/onboarding/self_hosted_entry_sheet.dart';
import 'package:buzz/features/onboarding/saved_identity_card.dart';
import 'package:buzz/shared/hosted/saved_identity_profile.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;

void main() {
  testWidgets('many saved identities share a cancellable lookup queue', (
    tester,
  ) async {
    final keys = [
      for (var i = 1; i <= 12; i++)
        nostr.Keys(i.toRadixString(16).padLeft(64, '0')),
    ];
    FlutterSecureStorage.setMockInitialValues({
      'buzz_communities': jsonEncode([
        for (final key in keys)
          {
            'name': 'Home',
            'relayUrl': 'wss://home.example',
            'pubkey': key.public,
            'nsec': key.nsec,
          },
      ]),
    });
    final clients = <_PendingProfileClient>[];
    final container = ProviderContainer(
      overrides: [
        savedIdentityProfileClientProvider.overrideWithValue(() {
          final client = _PendingProfileClient();
          clients.add(client);
          return client;
        }),
      ],
    );
    addTearDown(container.dispose);
    Widget chooser() => UncontrolledProviderScope(
      container: container,
      child: MaterialApp(
        home: const SelfHostedEntrySheet(
          relay: 'wss://join.example',
          name: 'Join',
          fullScreen: true,
        ),
      ),
    );
    await tester.pumpWidget(chooser());
    await tester.pumpAndSettle();
    expect(find.byType(SavedIdentityCard), findsNWidgets(12));
    expect(clients, hasLength(3));
    // Completing one lookup admits exactly one queued identity.
    clients.first.body.add(List.filled(256 * 1024 + 1, 32));
    await tester.pumpAndSettle();
    expect(clients, hasLength(4));
    expect(clients.first.closed, isTrue);
    // Leave while other requests and queued cards still exist.
    await tester.pumpWidget(
      UncontrolledProviderScope(container: container, child: const SizedBox()),
    );
    await tester.pumpAndSettle();
    expect(clients, hasLength(4));
    expect(
      clients.every((client) => client.closed && client.cancelled),
      isTrue,
    );
    await tester.pump(const Duration(seconds: 6));
    expect(clients, hasLength(4));
    // Reopening uses fresh work; cancelled slots cannot starve the chooser.
    await tester.pumpWidget(chooser());
    await tester.pumpAndSettle();
    expect(clients, hasLength(7));
    await tester.pumpWidget(
      UncontrolledProviderScope(container: container, child: const SizedBox()),
    );
    await tester.pumpAndSettle();
    expect(
      clients.every((client) => client.closed && client.cancelled),
      isTrue,
    );
  });
}

class _PendingProfileClient extends http.BaseClient {
  _PendingProfileClient() {
    body = StreamController<List<int>>(
      onCancel: () {
        cancelled = true;
      },
    );
  }
  late final StreamController<List<int>> body;
  bool closed = false;
  bool cancelled = false;

  @override
  Future<http.StreamedResponse> send(http.BaseRequest request) async =>
      http.StreamedResponse(body.stream, 200);

  @override
  void close() {
    closed = true;
    unawaited(body.close());
  }
}
