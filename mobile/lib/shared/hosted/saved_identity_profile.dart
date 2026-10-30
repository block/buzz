import 'dart:async';
import 'dart:convert';
import 'dart:collection';
import 'dart:isolate';
import 'dart:typed_data';

import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;

import '../crypto/event_signing.dart';
import '../profile/profile_event_parser.dart';
import '../profile/user_profile.dart';
import '../../features/age_gate/age_signal_provider.dart';
import '../relay/relay_query_response.dart';
import '../relay/relay_session.dart';
import 'device_identity_source.dart';
import 'hosted_relay_auth.dart';

final savedIdentityProfileClientProvider = Provider<http.Client Function()>(
  (ref) => http.Client.new,
);

final _profileQueueProvider = Provider((ref) => _ProfileQueue());

/// Fetches the saved identity's own profile without changing the active session.
final savedIdentityProfileProvider = FutureProvider.autoDispose
    .family<UserProfile?, String>((ref, pubkey) async {
      if (ref.watch(ageSignalProvider) == AgeSignalState.restricted) {
        return null;
      }
      final account = ref.read(builderlabAccountProvider);
      HostedRelayAuth? auth;
      final createTransport = ref.read(savedIdentityProfileClientProvider);
      final slot = ref.read(_profileQueueProvider).request();
      http.Client? transport;
      StreamIterator<List<int>>? chunks;
      var active = true;
      ref.onDispose(() {
        active = false;
        slot.cancelQueued();
        auth?.dispose();
        if (chunks != null) unawaited(chunks!.cancel());
        transport?.close();
      });
      if (!await slot.ready.future) return null;
      try {
        if (!active) return null;
        // Optional metadata reads must not clear or replace active reconnect
        // failures. Share the account session, but own assertion/recovery state.
        final profileAuth = HostedRelayAuth(account);
        auth = profileAuth;
        return await DeviceIdentitySource().loadProfile(pubkey, (config) async {
          if (!active) return null;
          final uri = Uri.parse(config.baseUrl).resolve('/query');
          final body = utf8.encode(
            jsonEncode([
              {
                'kinds': [0],
                'authors': [pubkey],
                'limit': 1,
              },
            ]),
          );
          final attemptTransport = createTransport();
          transport = attemptTransport;
          var requesting = true;
          chunks = null;
          final client = HostedRelayHttpClient(
            profileAuth,
            config,
            inner: attemptTransport,
            canSend: () => active && requesting,
          );
          final request = http.Request('POST', uri)
            ..followRedirects = false
            ..bodyBytes = body
            ..headers.addAll({
              'Content-Type': 'application/json',
              'Authorization': buildNip98AuthHeader(
                method: 'POST',
                url: uri.toString(),
                bodyBytes: body,
                nsec: config.nsec,
                pubkey: pubkey,
              ),
            });
          try {
            final response = await (() async {
              final response = await client.send(request);
              if (!active || !requesting) return null;
              chunks = StreamIterator(response.stream);
              // One kind-0 event should fit comfortably within 256 KiB.
              const maxBytes = 256 * 1024;
              if (response.statusCode != 200 ||
                  (response.contentLength ?? 0) > maxBytes) {
                return null;
              }
              final bytes = BytesBuilder(copy: false);
              while (await chunks!.moveNext()) {
                if (!active || !requesting) return null;
                final chunk = chunks!.current;
                if (chunk.length > maxBytes - bytes.length) return null;
                bytes.add(chunk);
              }
              if (!active || !requesting) return null;
              return (bytes.takeBytes(), response.headers['content-type']);
            })().timeout(const Duration(seconds: 5));
            if (!active || response == null) return null;
            // Retain the queue slot until bounded decoding actually finishes.
            return await _decodeProfile(response.$1, response.$2, pubkey);
          } finally {
            // Retire this attempt before another relay can be tried. A late auth
            // lookup must not send through the closed transport after timeout.
            requesting = false;
            if (chunks != null) unawaited(chunks!.cancel());
            client.close();
            transport = null;
          }
        });
      } finally {
        slot.release();
      }
    });

Future<UserProfile?> _decodeProfile(
  Uint8List bytes,
  String? contentType,
  String pubkey,
) => Isolate.run(() {
  // The transport normally inflates HTTP gzip before the byte cap. Reject
  // raw gzip here so the shared parser cannot expand a second, uncapped body.
  if (bytes.length > 2 && bytes[0] == 0x1f && bytes[1] == 0x8b) return null;
  final events = parseRelayQueryEvents(bytes, contentType);
  for (final event in events) {
    if (event.kind != 0 || event.pubkey != pubkey) continue;
    verifiedNostrEventFromJson(jsonEncode(event.toJson()));
    return parseProfileEvent(event).profile;
  }
  return null;
});

// Shared across identities in one provider container. Cancelled queued cards
// never reach secure storage, HTTP, or decoding.
class _ProfileQueue {
  final _waiting = Queue<_ProfileSlot>();
  int _running = 0;

  _ProfileSlot request() {
    final slot = _ProfileSlot(this);
    _waiting.add(slot);
    _drain();
    return slot;
  }

  void _drain() {
    while (_running < 3 && _waiting.isNotEmpty) {
      final slot = _waiting.removeFirst();
      _running++;
      slot.ready.complete(true);
    }
  }
}

class _ProfileSlot {
  _ProfileSlot(this.queue);
  final _ProfileQueue queue;
  final ready = Completer<bool>();
  bool _released = false;

  void cancelQueued() {
    if (ready.isCompleted) return;
    queue._waiting.remove(this);
    ready.complete(false);
    _released = true;
  }

  void release() {
    if (_released) return;
    _released = true;
    queue._running--;
    queue._drain();
  }
}
