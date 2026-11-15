// PROTOTYPE: headless Dart entrypoint for an Android push wake. Throwaway.
//
// Started from native (PushProtoReceiver) in a FlutterEngine with no UI.
// Reuses the app's CommunityStorage (flutter_secure_storage), RelayConfig and
// RelaySocket (NIP-42), then fetches the newest chat message and hands
// title/body back to native over a MethodChannel.
import 'dart:async';
import 'dart:convert';
import 'dart:ui';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';

import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/community/community_storage.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:buzz/shared/relay/relay_provider.dart';
import 'package:buzz/shared/relay/relay_socket.dart';

const _channel = MethodChannel('buzz/push_proto');

/// args: [elapsedRealtimeMinusEpochMs, mode, relayOverride, nsec]
/// mode: `seed` (store a community) or `wake` (fetch newest message).
@pragma('vm:entry-point')
Future<void> pushWakeMain(List<String> args) async {
  final offset = int.parse(args[0]);
  int now() => DateTime.now().millisecondsSinceEpoch + offset;
  final entered = now();
  WidgetsFlutterBinding.ensureInitialized();
  DartPluginRegistrant.ensureInitialized();
  void stage(String name, [int? t]) {
    _channel.invokeMethod<void>('stage', {'name': name, 't': t ?? now()});
  }

  stage('dart_entered', entered);
  try {
    stage('binding_ready');
    final mode = args[1];
    final relayOverride = args.length > 2 ? args[2] : '';
    if (mode == 'seed') {
      final storage = CommunityStorage();
      final community = Community.create(
        name: 'Proto',
        relayUrl: relayOverride,
        nsec: args[3],
      );
      await storage.save(community);
      await storage.saveActiveId(community.id);
      await _channel.invokeMethod<void>('seeded');
      return;
    }
    final message = await _fetchNewest(relayOverride, stage);
    await _channel.invokeMethod<void>('done', message);
  } catch (error, stack) {
    await _channel.invokeMethod<void>('fail', '$error\n$stack');
  }
}

Future<Map<String, String>> _fetchNewest(
  String relayOverride,
  void Function(String) stage,
) async {
  final storage = CommunityStorage();
  final communities = await storage.loadAll();
  final activeId = await storage.loadActiveId();
  stage('storage_loaded');
  final community = communities.firstWhere((c) => c.id == activeId);
  final config = RelayConfig(
    baseUrl: relayOverride.isEmpty ? community.relayUrl : relayOverride,
    nsec: community.nsec,
  );

  final event = Completer<Map<String, dynamic>?>();
  final profile = Completer<Map<String, dynamic>?>();
  final disconnected = Completer<void>(); // only ever completes with an error
  disconnected.future.ignore();
  Future<T> guard<T>(Future<T> f) => Future.any<T>([
    f,
    disconnected.future.then<T>((_) => throw StateError('unreachable')),
  ]);
  Object? lastError;
  late final RelaySocket socket;
  socket = RelaySocket(
    wsUrl: config.wsUrl,
    nsec: config.nsec,
    onMessage: (m) {
      final type = m[0] as String;
      final sub = m.length > 1 ? m[1] as String : '';
      if (type == 'EVENT') {
        final e = m[2] as Map<String, dynamic>;
        if (sub == 'newest' && !event.isCompleted) {
          stage('event_received');
          event.complete(e);
        } else if (sub == 'profile' && !profile.isCompleted) {
          stage('profile_received');
          profile.complete(e);
        }
      } else if (type == 'EOSE') {
        if (sub == 'newest' && !event.isCompleted) event.complete(null);
        if (sub == 'profile' && !profile.isCompleted) profile.complete(null);
      }
    },
    onConnected: () => stage('auth_done'),
    onDisconnected: (error) {
      lastError = error;
      if (!disconnected.isCompleted) {
        disconnected.completeError(error ?? 'relay disconnected');
      }
    },
  );
  socket.onOpen = () => stage('ws_connected');
  stage('connect_start');
  await guard(socket.connect());
  if (socket.state != SocketState.connected) {
    throw StateError('relay did not authenticate: $lastError');
  }

  socket.send([
    'REQ',
    'newest',
    {
      'kinds': [EventKind.streamMessage, EventKind.streamMessageV2],
      'limit': 1,
    },
  ]);
  stage('req_sent');
  final e = await guard(event.future);
  if (e == null) throw StateError('no message on relay');

  socket.send([
    'REQ',
    'profile',
    {
      'kinds': [EventKind.profile],
      'authors': [e['pubkey']],
      'limit': 1,
    },
  ]);
  final p = await guard(profile.future);
  var name = (e['pubkey'] as String).substring(0, 8);
  if (p != null) {
    final meta = jsonDecode(p['content'] as String) as Map<String, dynamic>;
    name = (meta['display_name'] ?? meta['name'] ?? name) as String;
  }
  unawaited(socket.disconnect());
  return {'title': name, 'body': e['content'] as String};
}
