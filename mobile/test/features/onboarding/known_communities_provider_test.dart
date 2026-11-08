import 'package:buzz/shared/hosted/known_community_sync_provider.dart';
import 'dart:async';

import 'package:buzz/shared/hosted/builderlab_account.dart';
import 'package:buzz/shared/hosted/hosted_environment.dart';
import 'package:buzz/shared/hosted/hosted_relay_auth.dart';
import 'package:buzz/shared/hosted/known_communities_provider.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _Account extends BuilderlabAccountClient {
  final subject = ValueNotifier<String?>('first');
  final firstResponse = Completer<Map<String, dynamic>>();
  final requested = <String>[];
  int restores = 0;

  @override
  ValueListenable<String?> get subjectChanges => subject;
  @override
  BuilderlabAccount? get account => subject.value == null
      ? null
      : BuilderlabAccount(
          subject: subject.value!,
          expiresAt: DateTime.now().add(const Duration(hours: 1)),
        );
  @override
  Future<BuilderlabAccount?> restore() async {
    restores++;
    return account;
  }

  @override
  Future<Map<String, dynamic>> post(
    String path,
    Map<String, dynamic> body, {
    String? expectedSubject,
  }) async {
    requested.add(path);
    if (subject.value == 'first') return firstResponse.future;
    return {
      'communities': [
        {'community_url': 'wss://second.communities.buzz.xyz'},
      ],
    };
  }
}

void main() {
  test(
    'discovery honors build boundary and discards old account results',
    () async {
      final account = _Account();
      final container = ProviderContainer(
        overrides: [
          builderlabAccountProvider.overrideWithValue(account),
          knownCommunitySyncProvider.overrideWithValue(null),
        ],
      );
      addTearDown(() {
        container.dispose();
        account.subject.dispose();
        account.dispose();
      });
      container.listen(knownCommunitiesProvider, (_, _) {});
      if (!HostedEnvironment.enabled) {
        expect(await container.read(knownCommunitiesProvider.future), isEmpty);
        expect(account.restores, 0);
        expect(account.requested, isEmpty);
        return;
      }
      await container.pump();
      expect(account.requested, ['/v1/buzz/known-communities/list']);
      account.subject.value = 'second';
      await container.pump();
      final current = await container.read(knownCommunitiesProvider.future);
      expect(current.single.relayUrl, 'wss://second.communities.buzz.xyz');
      account.firstResponse.complete({
        'communities': [
          {'community_url': 'wss://first.communities.buzz.xyz'},
        ],
      });
      await container.pump();
      expect(container.read(knownCommunitiesProvider).value, current);
      account.subject.value = null;
      await container.pump();
      expect(await container.read(knownCommunitiesProvider.future), isEmpty);
    },
  );
}
