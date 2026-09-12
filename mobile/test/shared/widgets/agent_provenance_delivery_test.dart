import 'dart:async';

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../../helpers/widget_helpers.dart';
import '../crypto/nip_oa_test.dart' show authTag, profile;

void main() {
  testWidgets(
    'mounted avatar follows relay authority and retired generations',
    (tester) async {
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(
            const MethodChannel(
              'dev.fluttercommunity.plus/connectivity_status',
            ),
            (_) async => null,
          );
      final owner = nostr.Keys.generate();
      final agent = nostr.Keys.generate();
      final peer = nostr.Keys.generate();
      final owned = profile(agent, [
        authTag(owner, agent.public),
      ], createdAt: 1);
      final revoked = profile(agent, [], createdAt: 2);
      var history = owned;
      final sockets = <_Socket>[];
      final config = _Config(owner.nsec);
      final session = RelaySessionNotifier(
        socketFactory:
            ({
              required wsUrl,
              required nsec,
              required onMessage,
              required onConnected,
              required onDisconnected,
            }) {
              final socket = _Socket(
                wsUrl: wsUrl,
                nsec: nsec,
                onMessage: onMessage,
                onConnected: onConnected,
                onDisconnected: onDisconnected,
                history: () => history,
                holdEose: sockets.isEmpty,
                directory: profile(agent, [], kind: 10100),
              );
              sockets.add(socket);
              return socket;
            },
      );
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            authProvider.overrideWith(_Auth.new),
            relayConfigProvider.overrideWith(() => config),
            relaySessionProvider.overrideWith(() => session),
          ],
          child: AvatarImage(
            imageUrl: null,
            radius: 24,
            fallback: const Text('A'),
            pubkey: agent.public,
            isAgent: true,
          ),
        ),
      );
      await tester.pumpAndSettle();
      void marker(bool visible) => expect(
        find.byIcon(LucideIcons.cloud),
        visible ? findsOneWidget : findsNothing,
      );
      final first = sockets.single;
      expect(first.profiles, isNotEmpty, reason: 'production must subscribe');
      await tester.pump(const Duration(milliseconds: 600));
      marker(false); // Acquisition timeout is not authoritative EOSE.
      for (final id in first.profiles) {
        first.receive(['EOSE', id]);
      }
      await tester.pumpAndSettle();
      marker(true); // Signed current-context positive before every denial.
      first.deliver(revoked);
      await tester.pumpAndSettle();
      marker(false);
      first.deliver(owned);
      await tester.pumpAndSettle();
      marker(false);
      first.drop();
      await tester.pump(const Duration(seconds: 1));
      await tester.pumpAndSettle();
      expect(
        sockets.length,
        2,
        reason: 'production reconnect creates a socket',
      );
      marker(false); // Reconnect history still contains the older owned event.
      final current = sockets.last;
      current.deliver(
        profile(agent, [authTag(owner, agent.public)], createdAt: 3),
      );
      await tester.pumpAndSettle();
      marker(true);
      for (final id in current.profiles.toList()) {
        current.receive(['CLOSED', id, 'restricted: no longer valid']);
      }
      await tester.pumpAndSettle();
      marker(
        false,
      ); // Positive display cache cannot bypass terminal feed failure.
      history = profile(agent, [], createdAt: 4);
      current.drop();
      await tester.pump(const Duration(seconds: 1));
      await tester.pumpAndSettle();
      marker(false);
      for (final next in [
        (url: 'https://one.example', owner: peer),
        (url: 'https://two.example', owner: peer),
      ]) {
        final retired = sockets.last;
        final ids = retired.profiles.toList();
        config.update(baseUrl: next.url, nsec: next.owner.nsec);
        await tester.pumpAndSettle();
        marker(false);
        expect(identical(sockets.last, retired), isFalse);
        final stale = profile(agent, [
          authTag(next.owner, agent.public),
        ], createdAt: 10);
        for (final id in ids) {
          retired.receive(['EVENT', id, stale.toJson()]);
        }
        await tester.pumpAndSettle();
        marker(false);
        sockets.last.deliver(
          profile(agent, [authTag(next.owner, agent.public)], createdAt: 11),
        );
        await tester.pumpAndSettle();
        marker(true); // New context can establish its own authority.
      }
      await tester.pumpWidget(const SizedBox());
      await tester.pumpAndSettle();
    },
  );
}

class _Auth extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.authenticated);
}

class _Config extends RelayConfigNotifier {
  _Config(this.key);
  final String key;
  @override
  RelayConfig build() => RelayConfig(baseUrl: 'https://one.example', nsec: key);
}

// Only the socket is controlled: production owns history, subscription readiness,
// event buffering, CLOSED dispatch, reconnect timers and generation fencing.
class _Socket extends RelaySocket {
  _Socket({
    required super.wsUrl,
    required super.nsec,
    required super.onMessage,
    required super.onConnected,
    required super.onDisconnected,
    required this.history,
    required this.holdEose,
    required this.directory,
  }) : receive = onMessage,
       connected = onConnected,
       disconnected = onDisconnected;
  final void Function(List<dynamic>) receive;
  final void Function() connected;
  final void Function(Object?) disconnected;
  final NostrEvent Function() history;
  final bool holdEose;
  final NostrEvent directory;
  final profiles = <String>{};
  @override
  Future<void> connect() async => connected();
  @override
  void dispose() {}
  void drop() => disconnected(null);
  void deliver(NostrEvent event) {
    for (final id in profiles.toList()) {
      receive(['EVENT', id, event.toJson()]);
    }
  }

  @override
  void send(List<dynamic> payload) {
    final id = payload[1] as String;
    if (payload.first == 'CLOSE') {
      profiles.remove(id);
      return;
    }
    if (payload.first != 'REQ') return;
    final kinds = (payload[2] as Map<String, dynamic>)['kinds'] as List;
    if (id.startsWith('l-') && kinds.contains(0)) profiles.add(id);
    scheduleMicrotask(() {
      if (kinds.contains(10100) || kinds.contains(0)) {
        final event = kinds.contains(10100) ? directory : history();
        receive(['EVENT', id, event.toJson()]);
      }
      if (!kinds.contains(0) || !holdEose) {
        receive(['EOSE', id]);
      }
    });
  }
}
