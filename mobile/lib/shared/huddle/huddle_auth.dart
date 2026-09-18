import 'package:flutter/foundation.dart';
import 'package:nostr/nostr.dart' as nostr;

import '../relay/nostr_models.dart';
import 'huddle_wire.dart';

/// Immutable connection inputs for one Huddle audio WebSocket.
@immutable
final class HuddleConnectionParameters {
  final String relayWebSocketUrl;
  final String nsec;
  final String parentChannelId;
  final String ephemeralChannelId;

  HuddleConnectionParameters({
    required this.relayWebSocketUrl,
    required this.nsec,
    required this.parentChannelId,
    required this.ephemeralChannelId,
  }) {
    final relayUri = Uri.tryParse(relayWebSocketUrl);
    if (relayUri == null ||
        !relayUri.hasAuthority ||
        (relayUri.scheme != 'ws' && relayUri.scheme != 'wss')) {
      throw ArgumentError.value(
        relayWebSocketUrl,
        'relayWebSocketUrl',
        'must be an absolute ws:// or wss:// relay URL',
      );
    }
    if (relayUri.hasQuery || relayUri.hasFragment) {
      throw ArgumentError.value(
        relayWebSocketUrl,
        'relayWebSocketUrl',
        'must not contain a query or fragment',
      );
    }
    _validateUuid(parentChannelId, 'parentChannelId');
    _validateUuid(ephemeralChannelId, 'ephemeralChannelId');
    if (nsec.trim().isEmpty) {
      throw ArgumentError.value(nsec, 'nsec', 'must not be empty');
    }
  }

  /// The endpoint is derived from the relay origin, but NIP-42's `relay` tag
  /// must remain [relayWebSocketUrl], not this Huddle-specific URL.
  Uri get audioWebSocketUri {
    final base = Uri.parse(relayWebSocketUrl);
    return base.replace(
      path: '/huddle/${Uri.encodeComponent(ephemeralChannelId)}/audio',
      query: null,
      fragment: null,
    );
  }
}

/// Fixed NIP-42 + Huddle auth envelope used after the audio relay challenge.
abstract final class HuddleAuthV2 {
  static Map<String, dynamic> buildMessage({
    required HuddleConnectionParameters parameters,
    required String challenge,
    int? createdAt,
  }) {
    if (challenge.isEmpty) {
      throw const HuddleAuthException('Relay challenge must not be empty.');
    }

    final secretKey = _decodeSecretKey(parameters.nsec);
    final event = nostr.Event.from(
      kind: EventKind.auth,
      content: '',
      tags: [
        ['relay', parameters.relayWebSocketUrl],
        ['challenge', challenge],
      ],
      secretKey: secretKey,
      createdAt: createdAt ?? DateTime.now().millisecondsSinceEpoch ~/ 1000,
      verify: false,
    );

    return {
      'type': 'auth',
      'event': event.toMap(),
      'parent_channel_id': parameters.parentChannelId,
      'protocol_version': HuddleWireV2.protocolVersion,
    };
  }

  static String _decodeSecretKey(String value) {
    final trimmed = value.trim();
    if (RegExp(r'^[0-9a-fA-F]{64}$').hasMatch(trimmed)) {
      return trimmed.toLowerCase();
    }
    try {
      final decoded = nostr.Nip19.decode(payload: trimmed).data;
      if (RegExp(r'^[0-9a-fA-F]{64}$').hasMatch(decoded)) {
        return decoded.toLowerCase();
      }
    } catch (_) {
      // Fall through to the stable domain error below.
    }
    throw const HuddleAuthException('Invalid Nostr signing key.');
  }
}

final class HuddleAuthException implements Exception {
  final String message;

  const HuddleAuthException(this.message);

  @override
  String toString() => 'HuddleAuthException: $message';
}

final _canonicalUuid = RegExp(
  r'^[0-9a-fA-F]{8}-[0-9a-fA-F]{4}-[0-9a-fA-F]{4}-'
  r'[0-9a-fA-F]{4}-[0-9a-fA-F]{12}$',
);

void _validateUuid(String value, String name) {
  if (!_canonicalUuid.hasMatch(value)) {
    throw ArgumentError.value(value, name, 'must be a canonical UUID');
  }
}
