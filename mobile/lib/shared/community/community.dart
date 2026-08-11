import 'package:uuid/uuid.dart';

import '../nostr/nostr_keys.dart';

const _uuid = Uuid();
const _sentinel = Object();

enum SensitiveActionPolicy { enabled, disabledByUser }

class Community {
  final String id;
  final String name;
  final String relayUrl;

  /// Canonical bech32 public identity. NIP-01 hex is derived only at the wire
  /// boundary and is never persisted here.
  final String? npub;
  final String? nsec;
  final SensitiveActionPolicy sensitiveActionPolicy;
  final DateTime addedAt;

  const Community({
    required this.id,
    required this.name,
    required this.relayUrl,
    this.npub,
    this.nsec,
    this.sensitiveActionPolicy = SensitiveActionPolicy.disabledByUser,
    required this.addedAt,
  });

  factory Community.create({
    required String name,
    required String relayUrl,
    String? npub,
    String? nsec,
    SensitiveActionPolicy sensitiveActionPolicy =
        SensitiveActionPolicy.disabledByUser,
  }) {
    final identity = tryIdentityFromStoredSecret(nsec);
    return Community(
      id: _uuid.v4(),
      name: name,
      relayUrl: relayUrl,
      npub: identity?.npub ?? tryNpubFromPublicKey(npub),
      nsec: identity?.nsec ?? nsec,
      sensitiveActionPolicy: sensitiveActionPolicy,
      addedAt: DateTime.now(),
    );
  }

  Community copyWith({
    String? name,
    String? relayUrl,
    Object? npub = _sentinel,
    Object? nsec = _sentinel,
    SensitiveActionPolicy? sensitiveActionPolicy,
  }) {
    return Community(
      id: id,
      name: name ?? this.name,
      relayUrl: relayUrl ?? this.relayUrl,
      npub: npub == _sentinel ? this.npub : npub as String?,
      nsec: nsec == _sentinel ? this.nsec : nsec as String?,
      sensitiveActionPolicy:
          sensitiveActionPolicy ?? this.sensitiveActionPolicy,
      addedAt: addedAt,
    );
  }

  Map<String, dynamic> toJson() => {
    'id': id,
    'name': name,
    'relayUrl': relayUrl,
    if (npub != null) 'npub': npub,
    if (nsec != null) 'nsec': nsec,
    'sensitiveActionPolicy': sensitiveActionPolicy.name,
    'addedAt': addedAt.toIso8601String(),
  };

  factory Community.fromJson(Map<String, dynamic> json) {
    final storedNsec = json['nsec'] as String?;
    final identity = tryIdentityFromStoredSecret(storedNsec);
    final storedPublicKey =
        json['npub'] as String? ?? json['pubkey'] as String?;
    return Community(
      id: json['id'] as String,
      name: json['name'] as String,
      relayUrl: json['relayUrl'] as String,
      // A valid secret is the source of truth and repairs stale/mismatched
      // legacy public-key fields during migration.
      npub: identity?.npub ?? tryNpubFromPublicKey(storedPublicKey),
      nsec: identity?.nsec ?? storedNsec,
      sensitiveActionPolicy: SensitiveActionPolicy.values.firstWhere(
        (value) => value.name == json['sensitiveActionPolicy'],
        orElse: () => SensitiveActionPolicy.disabledByUser,
      ),
      addedAt: DateTime.parse(json['addedAt'] as String),
    );
  }

  /// Derive a human-friendly community name from a relay URL.
  static String nameFromUrl(String url) {
    try {
      final host = Uri.parse(url).host;
      if (host.contains('localhost') || host == '127.0.0.1') return 'Local Dev';
      final parts = host.split('.');
      if (parts.length > 2) return parts.first;
      return host;
    } catch (_) {
      return 'Community';
    }
  }
}
