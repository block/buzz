import 'dart:async';
import 'dart:convert';
import 'dart:math';

import 'package:crypto/crypto.dart' as crypto;
import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;
import 'package:web_socket_channel/io.dart';

/// JOIN BY ADDRESS — a phone joins a community by pasting the relay's
/// wss:// URL alone: no desktop, no QR, no second machine.
///
/// The flow (the same protocol the web /join/ door rides, ported to the
/// phone):
///   1. connect to the PASTED url and REQ the community's owner-signed
///      JOIN MATERIAL event — kind 34550 (NIP-29's community-definition
///      kind, reused as the carrier; the only kind an unauthenticated REQ
///      may read). It carries the canonical origin, the standing invite
///      and the room list;
///   2. mint a key IN-POCKET (the stranger's key is absent → it is made
///      here; the user is never asked whether they "have a key");
///   3. claim the standing invite over HTTP, signing the NIP-98 header
///      with the CANONICAL url while TRANSPORTING on the pasted road
///      ("sign the identity, ride the road" — the alias would 401);
///   4. prove membership end-to-end: NIP-42 AUTH with the canonical
///      origin in the `relay` tag must come back OK true;
///   5. hand the joined [JoinResult] (community + identity) to the app's
///      normal community storage, which connects to the CANONICAL origin
///      from then on.
///
/// Fail-closed law: every refusal surfaces verbatim (the relay's CLOSED
/// reasons, the claim endpoint's error); nothing is guessed.

const int kJoinMaterialKind = 34550;
const Duration _fetchTimeout = Duration(seconds: 8);
const Duration _authTimeout = Duration(seconds: 12);

class JoinByAddressException implements Exception {
  final String message;
  const JoinByAddressException(this.message);

  @override
  String toString() => 'JoinByAddressException: $message';
}

/// The owner-signed join material as parsed off the event.
class JoinMaterial {
  final String canonicalHost;
  final String name;
  final String inviteCode;
  final Map<String, dynamic> raw;

  JoinMaterial({
    required this.canonicalHost,
    required this.name,
    required this.inviteCode,
    required this.raw,
  });
}

class JoinResult {
  final JoinMaterial material;
  final String nsec;
  final String pubkeyHex;

  JoinResult({
    required this.material,
    required this.nsec,
    required this.pubkeyHex,
  });
}

String _canonicalWs(JoinMaterial m) => 'wss://${m.canonicalHost}';
String _canonicalHttp(JoinMaterial m) => 'https://${m.canonicalHost}';

String _httpsOf(String wsUrl) => wsUrl.replaceFirst(RegExp(r'wss?://'), (wsUrl.startsWith('wss') ? 'https' : 'http'));

/// Parse the join event content — the v1 shape the relay's join.json also
/// carries. Malformed material = null (fail closed, never a guess).
JoinMaterial? parseJoinMaterial(String content) {
  final dynamic json;
  try {
    json = jsonDecode(content);
  } catch (_) {
    return null;
  }
  if (json is! Map<String, dynamic>) return null;
  final community = json['community'];
  final inviteUrl = json['invite_url'];
  if (community is! Map<String, dynamic>) return null;
  final host = community['host'];
  if (host is! String || host.isEmpty) return null;
  if (inviteUrl is! String || inviteUrl.isEmpty) return null;
  final marker = inviteUrl.indexOf('/invite/');
  if (marker < 0) return null;
  final code = inviteUrl
      .substring(marker + '/invite/'.length)
      .split(RegExp(r'[#?]'))[0];
  if (code.isEmpty) return null;
  final name = community['name'];
  return JoinMaterial(
    canonicalHost: host,
    name: name is String && name.isNotEmpty ? name : host,
    inviteCode: Uri.decodeComponent(code),
    raw: json,
  );
}

/// Fetch the join material event over the relay wire, unauthenticated.
Future<JoinMaterial?> fetchJoinMaterial(String wsUrl) async {
  final channel = IOWebSocketChannel.connect(Uri.parse(wsUrl));
  try {
    await channel.ready.timeout(_fetchTimeout);
    channel.sink.add(jsonEncode([
      'REQ',
      'join-material',
      {
        'kinds': [kJoinMaterialKind],
        'limit': 1,
      },
    ]));
    final completer = Completer<JoinMaterial?>();
    final sub = channel.stream.listen(
      (data) {
        final dynamic frame;
        try {
          frame = jsonDecode(data as String);
        } catch (_) {
          return;
        }
        if (frame is! List || frame.isEmpty) return;
        if (frame[0] == 'EVENT' && frame.length >= 3 && frame[2] is Map) {
          final event = frame[2] as Map<String, dynamic>;
          if (event['kind'] != kJoinMaterialKind) return;
          final material = parseJoinMaterial(
            event['content'] is String ? event['content'] as String : '',
          );
          if (!completer.isCompleted) completer.complete(material);
          return;
        }
        if (frame[0] == 'EOSE' || frame[0] == 'CLOSED') {
          if (!completer.isCompleted) completer.complete(null);
        }
      },
      onError: (Object e) {
        if (!completer.isCompleted) completer.complete(null);
      },
      onDone: () {
        if (!completer.isCompleted) completer.complete(null);
      },
    );
    final material = await completer.future.timeout(_fetchTimeout);
    await sub.cancel();
    return material;
  } catch (_) {
    return null;
  } finally {
    await channel.sink.close().timeout(
          const Duration(seconds: 2),
          onTimeout: () {},
        );
  }
}

/// Mint a fresh identity in-pocket: the stranger's key is absent, so one is
/// made here — the user is never asked whether they "have a key".
(String nsec, String pubkeyHex) _mintIdentity() {
  final keychain = nostr.Keys.generate();
  return (
    nostr.Nip19.encode(prefix: nostr.Nip19Prefix.nsec, data: keychain.secret),
    keychain.public,
  );
}

/// Claim the standing invite: POST {pasted origin}/api/invites/claim with
/// a NIP-98 header whose `u` tag names the CANONICAL url — transport on
/// the road the user pasted, signature on the community's identity.
Future<void> claimInvite({
  required String pastedWsUrl,
  required JoinMaterial material,
  required String privkeyHex,
  http.Client? httpClient,
}) async {
  final client = httpClient ?? http.Client();
  final canonicalClaimUrl =
      '${_canonicalHttp(material).replaceAll(RegExp(r'/+$'), '')}/api/invites/claim';
  final body = jsonEncode({'code': material.inviteCode});
  final payloadSha256 = crypto.sha256.convert(utf8.encode(body)).toString();
  final nonce = List<int>.generate(16, (_) => Random.secure().nextInt(256))
      .map((b) => b.toRadixString(16).padLeft(2, '0'))
      .join();
  final authEvent = nostr.Event.from(
    kind: 27235,
    content: '',
    tags: [
      ['u', canonicalClaimUrl],
      ['method', 'POST'],
      ['payload', payloadSha256],
      ['nonce', nonce],
    ],
    secretKey: privkeyHex,
  );
  final authorization =
      'Nostr ${base64Encode(utf8.encode(jsonEncode(authEvent.toMap())))}';
  final response = await client
      .post(
        Uri.parse(
          '${_httpsOf(pastedWsUrl).replaceAll(RegExp(r'/+$'), '')}/api/invites/claim',
        ),
        headers: {
          'Authorization': authorization,
          'Content-Type': 'application/json',
        },
        body: body,
      )
      .timeout(const Duration(seconds: 15));
  if (response.statusCode != 200) {
    String message = 'HTTP ${response.statusCode}';
    try {
      final decoded = jsonDecode(response.body);
      if (decoded is Map<String, dynamic> && decoded['error'] is String) {
        message = decoded['error'] as String;
      }
    } catch (_) {}
    throw JoinByAddressException('invite claim refused: $message');
  }
}

/// Prove the joined identity end-to-end: connect to the PASTED road and
/// complete NIP-42 AUTH with the CANONICAL origin in the `relay` tag
/// (signing the alias fails 401 — the founding law of this relay). The
/// relay's `OK true` for the AUTH event is the membership proof.
Future<void> verifyMembershipAuth({
  required String pastedWsUrl,
  required JoinMaterial material,
  required String privkeyHex,
}) async {
  final channel = IOWebSocketChannel.connect(Uri.parse(pastedWsUrl));
  try {
    await channel.ready.timeout(_authTimeout);
    final completer = Completer<bool>();
    String? authEventId;
    final sub = channel.stream.listen(
      (data) {
        final dynamic frame;
        try {
          frame = jsonDecode(data as String);
        } catch (_) {
          return;
        }
        if (frame is! List || frame.isEmpty) return;
        if (frame[0] == 'AUTH' && frame.length >= 2 && frame[1] is String) {
          final event = nostr.Event.from(
            kind: 22242,
            content: '',
            tags: [
              ['relay', _canonicalWs(material)],
              ['challenge', frame[1] as String],
            ],
            secretKey: privkeyHex,
          );
          authEventId = event.id;
          channel.sink.add(jsonEncode(['AUTH', event.toMap()]));
          return;
        }
        if (frame[0] == 'OK' && frame.length >= 3 && frame[1] == authEventId) {
          if (!completer.isCompleted) completer.complete(frame[2] == true);
        }
      },
      onError: (Object e) {
        if (!completer.isCompleted) completer.complete(false);
      },
      onDone: () {
        if (!completer.isCompleted) completer.complete(false);
      },
    );
    final ok = await completer.future.timeout(
      _authTimeout,
      onTimeout: () => false,
    );
    await sub.cancel();
    if (!ok) {
      throw const JoinByAddressException(
        'membership auth failed — the relay did not accept the key',
      );
    }
  } finally {
    await channel.sink.close().timeout(
          const Duration(seconds: 2),
          onTimeout: () {},
        );
  }
}

/// THE ENTRYPOINT: paste a wss:// URL, get back a joined community + the
/// in-pocket identity that joined it. Throws [JoinByAddressException]
/// with the relay's own refusal words on any failure — never a guess.
Future<JoinResult> joinByAddress(String pastedWsUrl) async {
  final normalized = pastedWsUrl.trim().replaceAll(RegExp(r'/+$'), '');
  final material = await fetchJoinMaterial(normalized);
  if (material == null) {
    throw const JoinByAddressException(
      'no join material published at that address '
      '(kind 34550 absent or malformed)',
    );
  }
  final (nsec, pubkeyHex) = _mintIdentity();
  final privkeyHex = nostr.Nip19.decode(payload: nsec).data;
  await claimInvite(
    pastedWsUrl: normalized,
    material: material,
    privkeyHex: privkeyHex,
  );
  await verifyMembershipAuth(
    pastedWsUrl: normalized,
    material: material,
    privkeyHex: privkeyHex,
  );
  return JoinResult(material: material, nsec: nsec, pubkeyHex: pubkeyHex);
}
