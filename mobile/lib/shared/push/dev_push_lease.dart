import 'dart:convert';
import 'dart:math';

import 'package:http/http.dart' as http;
import 'package:nostr/nostr.dart' as nostr;

import '../crypto/nip44.dart';
import '../relay/nostr_models.dart';
import '../relay/signed_event_relay.dart';
import 'push_bridge.dart';
import 'push_subscription.dart';

const buzzPushLeaseKind = 30350;
const buzzDevPushAppProfile = 'buzz-ios-dogfood';
const buzzPushTransport = 'apns';
const _maxSafeJsonInteger = 9007199254740991;
const _maxLeaseLifetimeSeconds = 2592000;
const _lowercaseHex64Pattern = r'^[0-9a-f]{64}$';
const _installationIdPattern = r'^[0-9a-f]{32}$';

class BuzzPushLeaseDescriptor {
  final String origin;
  final String executorKeyId;
  final String executorPubkey;
  final String transport;
  final int maxLeaseTtlSeconds;
  final int maxContentLength;
  final int maxPlaintextLength;
  final int maxEndpointLength;
  final int maxStringLength;

  const BuzzPushLeaseDescriptor({
    required this.origin,
    required this.executorKeyId,
    required this.executorPubkey,
    required this.transport,
    required this.maxLeaseTtlSeconds,
    required this.maxContentLength,
    required this.maxPlaintextLength,
    required this.maxEndpointLength,
    required this.maxStringLength,
  });

  factory BuzzPushLeaseDescriptor.fromRelayInformation(
    Map<String, dynamic> information,
  ) {
    _requireExactKeys(
      information,
      required: const {},
      allowed: const {
        'name',
        'description',
        'pubkey',
        'contact',
        'supported_nips',
        'supported_extensions',
        'software',
        'version',
        'limitation',
        'retention',
        'relay_countries',
        'language_tags',
        'tags',
        'posting_policy',
        'payments_url',
        'fees',
        'icon',
        'self',
        'pairing_relay_url',
        'push',
      },
      name: 'NIP-11 document',
    );
    final extensions = _stringList(
      information['supported_extensions'],
      name: 'supported_extensions',
    );
    if (!extensions.contains('nip-pl')) {
      throw const FormatException('NIP-11 does not advertise nip-pl');
    }

    final push = _stringMap(information['push'], name: 'push');
    _requireExactKeys(
      push,
      required: const {
        'origin',
        'keys',
        'app_profiles',
        'push_kinds',
        'h_grammar',
        'class_support',
        'limitation',
      },
      allowed: const {
        'origin',
        'keys',
        'app_profiles',
        'push_kinds',
        'h_grammar',
        'class_support',
        'limitation',
      },
      name: 'push descriptor',
    );

    final origin = _canonicalOrigin(push['origin']);
    final keys = _mapList(push['keys'], name: 'push.keys');
    final keyIds = <String>{};
    final currentKeys = <Map<String, dynamic>>[];
    for (final key in keys) {
      _requireExactKeys(
        key,
        required: const {'id', 'pubkey'},
        allowed: const {'id', 'pubkey', 'current', 'retiring'},
        name: 'push key',
      );
      final id = _nonEmptyString(key['id'], name: 'push key id');
      if (!keyIds.add(id)) {
        throw FormatException('Duplicate push key id: $id');
      }
      _lowercaseHex64(key['pubkey'], name: 'push key pubkey');
      final current = key['current'];
      final retiring = key['retiring'];
      if (current != null && current is! bool) {
        throw const FormatException('push key current must be a boolean');
      }
      if (retiring != null && retiring is! bool) {
        throw const FormatException('push key retiring must be a boolean');
      }
      if (current == true) currentKeys.add(key);
    }
    if (currentKeys.length != 1) {
      throw const FormatException(
        'push descriptor must contain exactly one current key',
      );
    }
    final currentKey = currentKeys.single;

    final profiles = _mapList(push['app_profiles'], name: 'app_profiles');
    final profileIds = <String>{};
    String? transport;
    for (final profile in profiles) {
      _requireExactKeys(
        profile,
        required: const {'id', 'transport'},
        allowed: const {'id', 'transport'},
        name: 'app profile',
      );
      final id = _nonEmptyString(profile['id'], name: 'app profile id');
      if (!profileIds.add(id)) {
        throw FormatException('Duplicate app profile id: $id');
      }
      final candidate = _nonEmptyString(
        profile['transport'],
        name: 'app profile transport',
      );
      if (id == buzzDevPushAppProfile) transport = candidate;
    }
    if (transport != buzzPushTransport) {
      throw const FormatException(
        'NIP-11 does not advertise the dogfood APNs profile',
      );
    }

    final pushKinds = _intList(push['push_kinds'], name: 'push_kinds');
    if (!buzzPushEligibleKinds.every(pushKinds.contains)) {
      throw const FormatException(
        'NIP-11 does not advertise every Buzz message kind for push',
      );
    }
    final hGrammar = _nonEmptyString(push['h_grammar'], name: 'h_grammar');
    if (hGrammar != 'uuid-v4-lowercase') {
      throw const FormatException('Unsupported push h_grammar');
    }

    final classSupport = _stringMap(
      push['class_support'],
      name: 'class_support',
    );
    final supportedClasses = _stringList(
      classSupport[buzzPushTransport],
      name: 'class_support.apns',
    );
    const knownClasses = {'default'};
    if (supportedClasses.any((value) => !knownClasses.contains(value))) {
      throw const FormatException('class_support contains an unknown class');
    }
    if (!supportedClasses.contains('default')) {
      throw const FormatException('APNs does not support the default class');
    }

    final limitation = _stringMap(push['limitation'], name: 'limitation');
    _requireExactKeys(
      limitation,
      required: const {
        'max_lease_ttl',
        'max_leases_per_pubkey',
        'max_subscriptions_per_lease',
        'max_kinds',
        'max_authors',
        'max_h',
        'max_tag_values',
        'max_ignore',
        'max_content_len',
        'max_plaintext_len',
        'max_endpoint_len',
        'max_string_len',
      },
      allowed: const {
        'max_lease_ttl',
        'max_leases_per_pubkey',
        'max_subscriptions_per_lease',
        'max_kinds',
        'max_authors',
        'max_h',
        'max_tag_values',
        'max_ignore',
        'max_content_len',
        'max_plaintext_len',
        'max_endpoint_len',
        'max_string_len',
      },
      name: 'push limitation',
    );
    for (final entry in limitation.entries) {
      _positiveInt(entry.value, name: 'limitation.${entry.key}');
    }
    final maxStringLength = limitation['max_string_len'] as int;
    _checkStringLength(origin, maxStringLength, name: 'origin');
    _checkStringLength(
      currentKey['id'] as String,
      maxStringLength,
      name: 'push key id',
    );
    final maxLeaseTtl = limitation['max_lease_ttl'] as int;
    if (maxLeaseTtl > _maxLeaseLifetimeSeconds) {
      throw const FormatException('max_lease_ttl exceeds the NIP-PL v1 limit');
    }

    return BuzzPushLeaseDescriptor(
      origin: origin,
      executorKeyId: currentKey['id'] as String,
      executorPubkey: currentKey['pubkey'] as String,
      transport: transport!,
      maxLeaseTtlSeconds: maxLeaseTtl,
      maxContentLength: limitation['max_content_len'] as int,
      maxPlaintextLength: limitation['max_plaintext_len'] as int,
      maxEndpointLength: limitation['max_endpoint_len'] as int,
      maxStringLength: maxStringLength,
    );
  }
}

Future<BuzzPushLeaseDescriptor> fetchBuzzPushLeaseDescriptor(
  String relayBaseUrl, {
  http.Client? client,
  Duration timeout = const Duration(seconds: 8),
}) async {
  final uri = Uri.tryParse(relayBaseUrl);
  if (uri == null ||
      !const {'http', 'https'}.contains(uri.scheme) ||
      uri.host.isEmpty) {
    throw FormatException('Invalid relay HTTP URL: $relayBaseUrl');
  }
  final requestUri = uri.resolve('/');
  final ownedClient = client ?? http.Client();
  try {
    final response = await ownedClient
        .get(requestUri, headers: const {'Accept': 'application/nostr+json'})
        .timeout(timeout);
    if (response.statusCode != 200) {
      throw StateError(
        'NIP-11 request failed with HTTP ${response.statusCode}: ${response.body}',
      );
    }
    final decoded = jsonDecode(response.body);
    if (decoded is! Map<String, dynamic>) {
      throw const FormatException('NIP-11 response must be a JSON object');
    }
    return BuzzPushLeaseDescriptor.fromRelayInformation(decoded);
  } finally {
    if (client == null) ownedClient.close();
  }
}

class BuzzPushLeasePublication {
  final String eventId;
  final int expiration;
  final String plaintext;

  const BuzzPushLeasePublication({
    required this.eventId,
    required this.expiration,
    required this.plaintext,
  });
}

typedef BuzzPushLeaseSubmit =
    Future<NostrEvent> Function({
      required int kind,
      required String content,
      required List<List<String>> tags,
      int? createdAt,
    });

Future<BuzzPushLeasePublication> publishBuzzDevPushLease({
  required BuzzPushEndpointGrant grant,
  String? leaseInstallationId,
  int? leaseGeneration,
  required BuzzPushLeaseDescriptor descriptor,
  required String nsec,
  required String memberPubkey,
  required List<BuzzPushSubscription> subscriptions,
  required BuzzPushLeaseSubmit submit,
  DateTime Function() now = DateTime.now,
}) async {
  _validateGrant(grant, descriptor);
  final effectiveLeaseInstallationId =
      leaseInstallationId ?? grant.installationId;
  if (!RegExp(_installationIdPattern).hasMatch(effectiveLeaseInstallationId)) {
    throw const FormatException(
      'Lease installation id must be 16 random bytes encoded as lowercase hex',
    );
  }
  final effectiveLeaseGeneration = leaseGeneration ?? grant.generation;
  if (effectiveLeaseGeneration <= 0 ||
      effectiveLeaseGeneration > _maxSafeJsonInteger) {
    throw const FormatException('Lease generation is invalid');
  }
  final normalizedMemberPubkey = _lowercaseHex64(
    memberPubkey,
    name: 'member pubkey',
  );
  final decoded = nostr.Nip19.decode(payload: nsec);
  if (decoded.prefix != nostr.Nip19Prefix.nsec || decoded.data.length != 64) {
    throw const FormatException('Signing key must be a 32-byte nsec');
  }
  final signingPubkey = nostr.Keys(decoded.data).public;
  if (signingPubkey != normalizedMemberPubkey) {
    throw const FormatException(
      'Authenticated signing key does not match the lease member pubkey',
    );
  }

  final nowSeconds = now().millisecondsSinceEpoch ~/ 1000;
  final expiration = min(
    grant.expiresAt,
    nowSeconds + descriptor.maxLeaseTtlSeconds,
  );
  if (expiration <= nowSeconds) {
    throw const FormatException('Endpoint grant is already expired');
  }

  final plaintextMap = <String, dynamic>{
    'v': 1,
    'origin': descriptor.origin,
    'app_profile': grant.appProfile,
    'transport': descriptor.transport,
    'endpoint': grant.endpointGrant,
    'generation': effectiveLeaseGeneration,
    'active': true,
    'subscriptions': [
      for (final subscription in subscriptions) subscription.toJson(),
    ],
  };
  final plaintext = jsonEncode(plaintextMap);
  if (utf8.encode(plaintext).length > descriptor.maxPlaintextLength) {
    throw const FormatException('lease plaintext exceeds the advertised limit');
  }
  final conversationKey = getConversationKey(
    decoded.data,
    descriptor.executorPubkey,
  );
  final content = nip44Encrypt(conversationKey, plaintext);
  if (utf8.encode(content).length > descriptor.maxContentLength) {
    throw const FormatException(
      'lease ciphertext exceeds the advertised limit',
    );
  }
  final acknowledged = await submit(
    kind: buzzPushLeaseKind,
    content: content,
    tags: [
      ['d', effectiveLeaseInstallationId],
      ['expiration', '$expiration'],
      ['exec', descriptor.executorKeyId],
    ],
    createdAt: nowSeconds,
  );
  if (acknowledged.id.isEmpty) {
    throw StateError('Relay returned an empty event id for the push lease');
  }
  return BuzzPushLeasePublication(
    eventId: acknowledged.id,
    expiration: expiration,
    plaintext: plaintext,
  );
}

Future<BuzzPushLeasePublication> publishBuzzDevPushLeaseThroughRelay({
  required BuzzPushEndpointGrant grant,
  String? leaseInstallationId,
  int? leaseGeneration,
  required BuzzPushLeaseDescriptor descriptor,
  required String nsec,
  required String memberPubkey,
  required List<BuzzPushSubscription> subscriptions,
  required SignedEventRelay relay,
  DateTime Function() now = DateTime.now,
}) => publishBuzzDevPushLease(
  grant: grant,
  leaseInstallationId: leaseInstallationId,
  leaseGeneration: leaseGeneration,
  descriptor: descriptor,
  nsec: nsec,
  memberPubkey: memberPubkey,
  subscriptions: subscriptions,
  submit: relay.submit,
  now: now,
);

/// Publishes the minimal higher-generation inactive lease used when a
/// community is removed from this device. Gateway delegation is intentionally
/// untouched because it is scoped to the installation and relay key, not the
/// community.
Future<BuzzPushLeasePublication> publishBuzzPushLeaseTombstone({
  required BuzzPushLeaseDescriptor descriptor,
  required String installationId,
  required int generation,
  required String nsec,
  required String memberPubkey,
  required BuzzPushLeaseSubmit submit,
  DateTime Function() now = DateTime.now,
}) async {
  if (!RegExp(_installationIdPattern).hasMatch(installationId)) {
    throw const FormatException(
      'Installation id must be 16 random bytes encoded as lowercase hex',
    );
  }
  if (generation <= 0 || generation > _maxSafeJsonInteger) {
    throw const FormatException('Tombstone generation is invalid');
  }
  final normalizedMemberPubkey = _lowercaseHex64(
    memberPubkey,
    name: 'member pubkey',
  );
  final decoded = nostr.Nip19.decode(payload: nsec);
  if (decoded.prefix != nostr.Nip19Prefix.nsec || decoded.data.length != 64) {
    throw const FormatException('Signing key must be a 32-byte nsec');
  }
  if (nostr.Keys(decoded.data).public != normalizedMemberPubkey) {
    throw const FormatException(
      'Authenticated signing key does not match the lease member pubkey',
    );
  }

  final nowSeconds = now().millisecondsSinceEpoch ~/ 1000;
  final expiration = nowSeconds + descriptor.maxLeaseTtlSeconds;
  final plaintextMap = <String, dynamic>{
    'v': 1,
    'origin': descriptor.origin,
    'generation': generation,
    'active': false,
  };
  final plaintext = jsonEncode(plaintextMap);
  if (utf8.encode(plaintext).length > descriptor.maxPlaintextLength) {
    throw const FormatException('lease plaintext exceeds the advertised limit');
  }
  final content = nip44Encrypt(
    getConversationKey(decoded.data, descriptor.executorPubkey),
    plaintext,
  );
  if (utf8.encode(content).length > descriptor.maxContentLength) {
    throw const FormatException(
      'lease ciphertext exceeds the advertised limit',
    );
  }
  final acknowledged = await submit(
    kind: buzzPushLeaseKind,
    content: content,
    tags: [
      ['d', installationId],
      ['expiration', '$expiration'],
      ['exec', descriptor.executorKeyId],
    ],
    createdAt: nowSeconds,
  );
  if (acknowledged.id.isEmpty) {
    throw StateError('Relay returned an empty event id for the push tombstone');
  }
  return BuzzPushLeasePublication(
    eventId: acknowledged.id,
    expiration: expiration,
    plaintext: plaintext,
  );
}

Future<BuzzPushLeasePublication> publishBuzzPushLeaseTombstoneThroughRelay({
  required BuzzPushLeaseDescriptor descriptor,
  required String installationId,
  required int generation,
  required String nsec,
  required String memberPubkey,
  required SignedEventRelay relay,
  DateTime Function() now = DateTime.now,
}) => publishBuzzPushLeaseTombstone(
  descriptor: descriptor,
  installationId: installationId,
  generation: generation,
  nsec: nsec,
  memberPubkey: memberPubkey,
  submit: relay.submit,
  now: now,
);

void _validateGrant(
  BuzzPushEndpointGrant grant,
  BuzzPushLeaseDescriptor descriptor,
) {
  if (utf8.encode(grant.installationId).length > 64 ||
      !RegExp(_installationIdPattern).hasMatch(grant.installationId)) {
    throw const FormatException(
      'Installation id must be 16 random bytes encoded as lowercase hex',
    );
  }
  if (grant.relayPubkey != descriptor.executorPubkey) {
    throw const FormatException(
      'Stored endpoint grant is delegated to a different relay key',
    );
  }
  if (grant.appProfile != buzzDevPushAppProfile) {
    throw const FormatException('Endpoint grant is not for buzz-ios-dogfood');
  }
  if (grant.endpointGrant.isEmpty ||
      utf8.encode(grant.endpointGrant).length > descriptor.maxEndpointLength) {
    throw const FormatException(
      'Endpoint grant violates the advertised endpoint limit',
    );
  }
  if (grant.endpointEpoch <= 0) {
    throw const FormatException('Endpoint grant epoch is invalid');
  }
  if (grant.generation <= 0 || grant.generation > _maxSafeJsonInteger) {
    throw const FormatException('Endpoint grant generation is invalid');
  }
}

String _canonicalOrigin(Object? value) {
  final origin = _nonEmptyString(value, name: 'origin');
  final uri = Uri.tryParse(origin);
  if (uri == null ||
      !const {'ws', 'wss'}.contains(uri.scheme) ||
      uri.host.isEmpty ||
      uri.userInfo.isNotEmpty ||
      uri.path.isNotEmpty ||
      uri.hasQuery ||
      uri.hasFragment ||
      '${uri.scheme}://${uri.authority}' != origin) {
    throw FormatException('Invalid canonical push origin: $origin');
  }
  return origin;
}

void _checkStringLength(String value, int maximum, {required String name}) {
  if (maximum <= 0 || utf8.encode(value).length > maximum) {
    throw FormatException('$name exceeds its advertised byte limit');
  }
}

String _lowercaseHex64(Object? value, {required String name}) {
  final text = _nonEmptyString(value, name: name);
  if (!RegExp(_lowercaseHex64Pattern).hasMatch(text)) {
    throw FormatException('$name must be exactly 64 lowercase hex characters');
  }
  return text;
}

String _nonEmptyString(Object? value, {required String name}) {
  if (value is! String || value.isEmpty) {
    throw FormatException('$name must be a non-empty string');
  }
  return value;
}

int _positiveInt(Object? value, {required String name}) {
  if (value is! int || value <= 0) {
    throw FormatException('$name must be a positive integer');
  }
  return value;
}

Map<String, dynamic> _stringMap(Object? value, {required String name}) {
  if (value is! Map<String, dynamic>) {
    throw FormatException('$name must be an object');
  }
  return value;
}

List<Map<String, dynamic>> _mapList(Object? value, {required String name}) {
  if (value is! List || value.isEmpty) {
    throw FormatException('$name must be a non-empty array');
  }
  return [
    for (final item in value)
      if (item is Map<String, dynamic>)
        item
      else
        throw FormatException('$name entries must be objects'),
  ];
}

List<String> _stringList(Object? value, {required String name}) {
  if (value is! List || value.isEmpty) {
    throw FormatException('$name must be a non-empty string array');
  }
  return [
    for (final item in value)
      if (item is String && item.isNotEmpty)
        item
      else
        throw FormatException('$name entries must be non-empty strings'),
  ];
}

List<int> _intList(
  Object? value, {
  required String name,
  bool allowEmpty = false,
}) {
  if (value is! List || (!allowEmpty && value.isEmpty)) {
    throw FormatException(
      '$name must be ${allowEmpty ? 'an' : 'a non-empty'} integer array',
    );
  }
  return [
    for (final item in value)
      if (item is int && item >= 0)
        item
      else
        throw FormatException('$name entries must be non-negative integers'),
  ];
}

void _requireExactKeys(
  Map<String, dynamic> value, {
  required Set<String> required,
  required Set<String> allowed,
  required String name,
}) {
  final missing = required.difference(value.keys.toSet());
  if (missing.isNotEmpty) {
    throw FormatException('$name is missing ${missing.join(', ')}');
  }
  final unexpected = value.keys.toSet().difference(allowed);
  if (unexpected.isNotEmpty) {
    throw FormatException('$name contains unknown field ${unexpected.first}');
  }
}
