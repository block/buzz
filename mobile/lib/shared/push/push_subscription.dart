import 'dart:convert';

/// User-visible Buzz message kinds. This mirrors
/// `EventKind.channelMessageEventKinds` without importing feature code into
/// the shared push layer.
const buzzPushEligibleKinds = [9, 40002, 45001, 45003];
const buzzPushSelfDirectedKinds = buzzPushEligibleKinds;
const buzzPushRenderableKinds = buzzPushEligibleKinds;
const buzzPushChannelKinds = [9];
const buzzPushChannelChunkSize = 50;
const buzzPushMaxSubscriptions = 16;
const buzzPushMaxIgnoreFilters = 8;
const buzzPushHellthreadParticipantLimit = 20;

const _supportedNotificationClasses = {'default'};
const _filterKeys = {'kinds', 'authors', '#p', '#h', '#e'};
final _exactHexPattern = RegExp(r'^[0-9a-f]{64}$');
final _channelIdPattern = RegExp(
  r'^[0-9a-f]{8}-[0-9a-f]{4}-4[0-9a-f]{3}-[89ab][0-9a-f]{3}-[0-9a-f]{12}$',
);

enum BuzzPushLeaseSubscriptionAuthority { desired, accepted }

class BuzzPushFilter {
  final List<int> kinds;
  final List<String>? authors;
  final List<String>? pTags;
  final List<String>? hTags;
  final List<String>? eTags;

  BuzzPushFilter({
    required Iterable<int> kinds,
    Iterable<String>? authors,
    Iterable<String>? pTags,
    Iterable<String>? hTags,
    Iterable<String>? eTags,
  }) : kinds = List.unmodifiable(kinds),
       authors = _optionalList(authors),
       pTags = _optionalList(pTags),
       hTags = _optionalList(hTags),
       eTags = _optionalList(eTags) {
    _validate();
  }

  Map<String, dynamic> toJson() => {
    'kinds': kinds,
    if (authors != null) 'authors': authors,
    if (pTags != null) '#p': pTags,
    if (hTags != null) '#h': hTags,
    if (eTags != null) '#e': eTags,
  };

  factory BuzzPushFilter.fromJson(Map<String, dynamic> json) {
    _rejectUnknownKeys(json, _filterKeys, 'push filter');
    return BuzzPushFilter(
      kinds: _intList(json, 'kinds'),
      authors: _optionalStringList(json, 'authors'),
      pTags: _optionalStringList(json, '#p'),
      hTags: _optionalStringList(json, '#h'),
      eTags: _optionalStringList(json, '#e'),
    );
  }

  void _validate() {
    if (kinds.isEmpty ||
        kinds.any((kind) => !buzzPushEligibleKinds.contains(kind))) {
      throw const FormatException('Push filter contains invalid kinds.');
    }
    for (final value in [...?authors, ...?pTags, ...?eTags]) {
      if (!_exactHexPattern.hasMatch(value)) {
        throw const FormatException(
          'Push filter contains a non-exact hex value.',
        );
      }
    }
    for (final value in hTags ?? const <String>[]) {
      if (!_channelIdPattern.hasMatch(value)) {
        throw const FormatException(
          'Push filter contains an invalid channel ID.',
        );
      }
    }
  }
}

class BuzzPushSuppression {
  final int pTagsMax;

  const BuzzPushSuppression({required this.pTagsMax}) : assert(pTagsMax > 0);

  Map<String, dynamic> toJson() => {'p_tags_max': pTagsMax};

  factory BuzzPushSuppression.fromJson(Map<String, dynamic> json) {
    _rejectUnknownKeys(json, const {'p_tags_max'}, 'push suppression');
    final value = json['p_tags_max'];
    if (value is! int || value <= 0) {
      throw const FormatException('p_tags_max must be a positive integer.');
    }
    return BuzzPushSuppression(pTagsMax: value);
  }
}

class BuzzPushSubscription {
  final BuzzPushFilter filter;
  final String notificationClass;
  final List<BuzzPushFilter> ignore;
  final BuzzPushSuppression? suppress;

  BuzzPushSubscription({
    required this.filter,
    required this.notificationClass,
    Iterable<BuzzPushFilter> ignore = const [],
    this.suppress,
  }) : ignore = List.unmodifiable(ignore) {
    if (!_supportedNotificationClasses.contains(notificationClass)) {
      throw const FormatException('Unsupported push notification class.');
    }
    if (filter.authors == null &&
        filter.pTags == null &&
        filter.hTags == null) {
      throw const FormatException('Push subscription filter is not narrowed.');
    }
    if (this.ignore.length > buzzPushMaxIgnoreFilters) {
      throw const FormatException(
        'Push subscription has too many ignore filters.',
      );
    }
  }

  Map<String, dynamic> toJson() => {
    'filter': filter.toJson(),
    'class': notificationClass,
    if (ignore.isNotEmpty)
      'ignore': [for (final filter in ignore) filter.toJson()],
    if (suppress != null) 'suppress': suppress!.toJson(),
  };

  factory BuzzPushSubscription.fromJson(Map<String, dynamic> json) {
    _rejectUnknownKeys(json, const {
      'filter',
      'class',
      'ignore',
      'suppress',
    }, 'push subscription');
    final filter = json['filter'];
    final notificationClass = json['class'];
    final ignore = json['ignore'];
    final suppress = json['suppress'];
    if (filter is! Map || notificationClass is! String) {
      throw const FormatException('Malformed push subscription.');
    }
    if (ignore != null && ignore is! List) {
      throw const FormatException('Push subscription ignore must be a list.');
    }
    if (suppress != null && suppress is! Map) {
      throw const FormatException(
        'Push subscription suppress must be an object.',
      );
    }
    return BuzzPushSubscription(
      filter: BuzzPushFilter.fromJson(Map<String, dynamic>.from(filter)),
      notificationClass: notificationClass,
      ignore: [
        for (final raw in ignore as List<dynamic>? ?? const [])
          if (raw is Map)
            BuzzPushFilter.fromJson(Map<String, dynamic>.from(raw))
          else
            throw const FormatException('Malformed push ignore filter.'),
      ],
      suppress: suppress == null
          ? null
          : BuzzPushSuppression.fromJson(Map<String, dynamic>.from(suppress)),
    );
  }
}

/// Desired and relay-accepted lease subscriptions are intentionally separate.
/// Keeping both sets makes relay rejection or expiry detectable instead of
/// assuming the desired lease was accepted unchanged.
class BuzzPushLeaseSubscriptionState {
  final BuzzPushLeaseSubscriptionAuthority authority;
  final List<BuzzPushSubscription> desired;
  final List<BuzzPushSubscription>? accepted;

  /// Monotonic generation of the relay-facing kind-30350 lease.
  final int? acceptedGeneration;

  /// Highest lease generation durably reserved by the client. This advances
  /// before relay publication so a relay commit followed by a local failure
  /// cannot make the next retry reuse a stale generation.
  final int? generationCursor;

  /// Higher-generation inactive lease that still needs relay acceptance.
  ///
  /// This is persisted before publication. Ambiguous retries reserve a newer
  /// generation so a relay-accepted tombstone whose local acknowledgement was
  /// lost is safely superseded without weakening strict relay monotonicity.
  final int? pendingTombstoneGeneration;

  const BuzzPushLeaseSubscriptionState.desired({
    this.desired = const [],
    this.accepted,
    this.acceptedGeneration,
    this.generationCursor,
    this.pendingTombstoneGeneration,
  }) : assert(
         pendingTombstoneGeneration == null ||
             (pendingTombstoneGeneration > (acceptedGeneration ?? 0) &&
                 generationCursor != null &&
                 pendingTombstoneGeneration <= generationCursor),
       ),
       authority = BuzzPushLeaseSubscriptionAuthority.desired;

  BuzzPushLeaseSubscriptionState.accepted({
    required Iterable<BuzzPushSubscription> desired,
    required Iterable<BuzzPushSubscription> acceptedSubscriptions,
    required this.acceptedGeneration,
    this.generationCursor,
    this.pendingTombstoneGeneration,
  }) : authority = BuzzPushLeaseSubscriptionAuthority.accepted,
       desired = List.unmodifiable(desired),
       accepted = List.unmodifiable(acceptedSubscriptions) {
    if (acceptedGeneration == null || acceptedGeneration! <= 0) {
      throw const FormatException(
        'Accepted push authority requires a positive lease generation.',
      );
    }
    if (generationCursor != null && generationCursor! < acceptedGeneration!) {
      throw const FormatException(
        'Push lease generation cursor cannot trail the accepted generation.',
      );
    }
    _validatePendingTombstone();
  }

  void _validatePendingTombstone() {
    final pending = pendingTombstoneGeneration;
    if (pending == null) return;
    if (pending <= (acceptedGeneration ?? 0) ||
        generationCursor == null ||
        pending > generationCursor!) {
      throw const FormatException(
        'Pending push tombstone must be newer than accepted state and durably reserved.',
      );
    }
  }

  List<BuzzPushSubscription> get authoritative => switch (authority) {
    BuzzPushLeaseSubscriptionAuthority.desired => desired,
    BuzzPushLeaseSubscriptionAuthority.accepted => accepted!,
  };

  BuzzPushLeaseSubscriptionState withDesired(
    Iterable<BuzzPushSubscription> subscriptions,
  ) {
    final updated = List<BuzzPushSubscription>.unmodifiable(subscriptions);
    return switch (authority) {
      BuzzPushLeaseSubscriptionAuthority.desired =>
        BuzzPushLeaseSubscriptionState.desired(
          desired: updated,
          accepted: accepted,
          acceptedGeneration: acceptedGeneration,
          generationCursor: generationCursor,
          pendingTombstoneGeneration: pendingTombstoneGeneration,
        ),
      BuzzPushLeaseSubscriptionAuthority.accepted =>
        BuzzPushLeaseSubscriptionState.accepted(
          desired: updated,
          acceptedSubscriptions: accepted!,
          acceptedGeneration: acceptedGeneration,
          generationCursor: generationCursor,
          pendingTombstoneGeneration: pendingTombstoneGeneration,
        ),
    };
  }

  BuzzPushLeaseSubscriptionState withAccepted({
    required Iterable<BuzzPushSubscription> subscriptions,
    required int generation,
  }) => BuzzPushLeaseSubscriptionState.accepted(
    desired: desired,
    acceptedSubscriptions: subscriptions,
    acceptedGeneration: generation,
    generationCursor: generationCursor == null || generation > generationCursor!
        ? generation
        : generationCursor,
    pendingTombstoneGeneration:
        pendingTombstoneGeneration != null &&
            generation < pendingTombstoneGeneration!
        ? pendingTombstoneGeneration
        : null,
  );

  BuzzPushLeaseSubscriptionState withReservedGeneration(int generation) {
    if (generation <= (generationCursor ?? acceptedGeneration ?? 0)) {
      throw const FormatException(
        'Reserved push lease generation must advance monotonically.',
      );
    }
    return switch (authority) {
      BuzzPushLeaseSubscriptionAuthority.desired =>
        BuzzPushLeaseSubscriptionState.desired(
          desired: desired,
          accepted: accepted,
          acceptedGeneration: acceptedGeneration,
          generationCursor: generation,
          pendingTombstoneGeneration: pendingTombstoneGeneration,
        ),
      BuzzPushLeaseSubscriptionAuthority.accepted =>
        BuzzPushLeaseSubscriptionState.accepted(
          desired: desired,
          acceptedSubscriptions: accepted!,
          acceptedGeneration: acceptedGeneration,
          generationCursor: generation,
          pendingTombstoneGeneration: pendingTombstoneGeneration,
        ),
    };
  }

  BuzzPushLeaseSubscriptionState withPendingTombstone(int generation) {
    if (generation <= (generationCursor ?? acceptedGeneration ?? 0)) {
      throw const FormatException(
        'Pending push tombstone generation must advance monotonically.',
      );
    }
    return BuzzPushLeaseSubscriptionState.desired(
      desired: desired,
      accepted: accepted,
      acceptedGeneration: acceptedGeneration,
      generationCursor: generation,
      pendingTombstoneGeneration: generation,
    );
  }

  /// Migrates a generation reserved by an older client before the explicit
  /// tombstone journal field existed.
  BuzzPushLeaseSubscriptionState withPendingTombstoneAtCursor() {
    final generation = generationCursor;
    if (generation == null || generation <= (acceptedGeneration ?? 0)) {
      return this;
    }
    return BuzzPushLeaseSubscriptionState.desired(
      desired: desired,
      accepted: accepted,
      acceptedGeneration: acceptedGeneration,
      generationCursor: generation,
      pendingTombstoneGeneration: generation,
    );
  }

  BuzzPushLeaseSubscriptionState withAcceptedTombstone(int generation) {
    if (generation != pendingTombstoneGeneration ||
        generation < (acceptedGeneration ?? 0)) {
      return this;
    }
    return BuzzPushLeaseSubscriptionState.desired(
      desired: desired,
      acceptedGeneration: generation,
      generationCursor:
          generationCursor == null || generation > generationCursor!
          ? generation
          : generationCursor,
    );
  }

  Map<String, dynamic> toJson() => {
    'authority': authority.name,
    'desired': [for (final subscription in desired) subscription.toJson()],
    if (accepted != null)
      'accepted': [for (final subscription in accepted!) subscription.toJson()],
    if (acceptedGeneration != null) 'acceptedGeneration': acceptedGeneration,
    if (generationCursor != null) 'generationCursor': generationCursor,
    if (pendingTombstoneGeneration != null)
      'pendingTombstoneGeneration': pendingTombstoneGeneration,
  };

  factory BuzzPushLeaseSubscriptionState.fromJson(Map<String, dynamic> json) {
    _rejectUnknownKeys(json, const {
      'authority',
      'desired',
      'accepted',
      'acceptedGeneration',
      'generationCursor',
      'pendingTombstoneGeneration',
    }, 'push subscription state');
    final authority = json['authority'];
    final desired = _subscriptionList(
      json['desired'],
      'desired',
      allowEmpty: authority == 'desired',
    );
    final acceptedRaw = json['accepted'];
    final accepted = acceptedRaw == null
        ? null
        : _subscriptionList(acceptedRaw, 'accepted');
    final acceptedGeneration = json['acceptedGeneration'];
    final generationCursor = json['generationCursor'];
    final pendingTombstoneGeneration = json['pendingTombstoneGeneration'];
    if (acceptedGeneration != null && acceptedGeneration is! int) {
      throw const FormatException(
        'Accepted push lease generation must be an integer.',
      );
    }
    if (generationCursor != null && generationCursor is! int) {
      throw const FormatException(
        'Push lease generation cursor must be an integer.',
      );
    }
    if (pendingTombstoneGeneration != null &&
        pendingTombstoneGeneration is! int) {
      throw const FormatException(
        'Pending push tombstone generation must be an integer.',
      );
    }
    if (authority == 'desired' &&
        pendingTombstoneGeneration is int &&
        (pendingTombstoneGeneration <= (acceptedGeneration as int? ?? 0) ||
            generationCursor is! int ||
            pendingTombstoneGeneration > generationCursor)) {
      throw const FormatException(
        'Pending push tombstone must be newer than accepted state and durably reserved.',
      );
    }
    return switch (authority) {
      'desired' => BuzzPushLeaseSubscriptionState.desired(
        desired: desired,
        accepted: accepted,
        acceptedGeneration: acceptedGeneration as int?,
        generationCursor: generationCursor as int?,
        pendingTombstoneGeneration: pendingTombstoneGeneration as int?,
      ),
      'accepted' when accepted != null && acceptedGeneration is int =>
        BuzzPushLeaseSubscriptionState.accepted(
          desired: desired,
          acceptedSubscriptions: accepted,
          acceptedGeneration: acceptedGeneration,
          generationCursor: generationCursor as int?,
          pendingTombstoneGeneration: pendingTombstoneGeneration as int?,
        ),
      'accepted' => throw const FormatException(
        'Accepted push authority requires accepted subscriptions and generations.',
      ),
      _ => throw const FormatException('Unknown push subscription authority.'),
    };
  }
}

List<BuzzPushSubscription> buildDesiredBuzzPushSubscriptions({
  required String myPubkey,
  Iterable<String> channelIds = const [],
  Iterable<String> mutedChannelIds = const [],
}) {
  final normalizedPubkey = myPubkey.toLowerCase();
  if (!_exactHexPattern.hasMatch(normalizedPubkey)) {
    throw const FormatException('Push subscription pubkey must be exact hex.');
  }

  final normalizedChannelIds = channelIds.map(_normalizeChannelID).toSet();
  final normalizedMuted = mutedChannelIds.map(_normalizeChannelID).toSet();
  final activeMuted =
      normalizedMuted.intersection(normalizedChannelIds).toList()..sort();
  final mutedIgnoreFilters = <BuzzPushFilter>[];
  for (final chunk in _chunks(activeMuted, buzzPushChannelChunkSize)) {
    mutedIgnoreFilters.add(
      BuzzPushFilter(kinds: buzzPushChannelKinds, hTags: chunk),
    );
  }
  if (mutedIgnoreFilters.length + 1 > buzzPushMaxIgnoreFilters) {
    throw const FormatException('Too many muted channels for a push lease.');
  }

  final selfAuthored = BuzzPushFilter(
    kinds: buzzPushRenderableKinds,
    authors: [normalizedPubkey],
  );
  final ignores = [selfAuthored, ...mutedIgnoreFilters];
  const suppression = BuzzPushSuppression(
    pTagsMax: buzzPushHellthreadParticipantLimit,
  );
  final subscriptions = <BuzzPushSubscription>[
    BuzzPushSubscription(
      filter: BuzzPushFilter(
        kinds: buzzPushSelfDirectedKinds,
        pTags: [normalizedPubkey],
      ),
      notificationClass: 'default',
      ignore: ignores,
      suppress: suppression,
    ),
  ];

  final channels = normalizedChannelIds.difference(normalizedMuted).toList()
    ..sort();
  for (final chunk in _chunks(channels, buzzPushChannelChunkSize)) {
    subscriptions.add(
      BuzzPushSubscription(
        filter: BuzzPushFilter(kinds: buzzPushChannelKinds, hTags: chunk),
        notificationClass: 'default',
        ignore: ignores,
        suppress: suppression,
      ),
    );
  }
  if (subscriptions.length > buzzPushMaxSubscriptions) {
    throw const FormatException('Too many channels for a push lease.');
  }
  return List.unmodifiable(subscriptions);
}

String buzzPushSubscriptionsFingerprint(
  List<BuzzPushSubscription> subscriptions,
) => jsonEncode([
  for (final subscription in subscriptions) subscription.toJson(),
]);

String buzzPushSubscriptionStateFingerprint(
  BuzzPushLeaseSubscriptionState state,
) => jsonEncode(state.toJson());

List<List<String>> _chunks(List<String> values, int size) => [
  for (var offset = 0; offset < values.length; offset += size)
    values.sublist(offset, (offset + size).clamp(0, values.length)),
];

String _normalizeChannelID(String value) {
  final normalized = value.toLowerCase();
  if (!_channelIdPattern.hasMatch(normalized)) {
    throw const FormatException('Push subscription channel ID is invalid.');
  }
  return normalized;
}

List<String>? _optionalList(Iterable<String>? values) =>
    values == null ? null : List.unmodifiable(values);

List<int> _intList(Map<String, dynamic> json, String key) {
  final raw = json[key];
  if (raw is! List || raw.any((value) => value is! int)) {
    throw FormatException('$key must be an integer list.');
  }
  return raw.cast<int>();
}

List<String>? _optionalStringList(Map<String, dynamic> json, String key) {
  if (!json.containsKey(key)) return null;
  final raw = json[key];
  if (raw is! List || raw.isEmpty || raw.any((value) => value is! String)) {
    throw FormatException('$key must be a non-empty string list.');
  }
  return raw.cast<String>();
}

List<BuzzPushSubscription> _subscriptionList(
  Object? raw,
  String label, {
  bool allowEmpty = false,
}) {
  if (raw is! List || (!allowEmpty && raw.isEmpty)) {
    throw FormatException('$label subscriptions must be a non-empty list.');
  }
  return [
    for (final item in raw)
      if (item is Map)
        BuzzPushSubscription.fromJson(Map<String, dynamic>.from(item))
      else
        throw FormatException('Malformed $label subscription.'),
  ];
}

void _rejectUnknownKeys(
  Map<String, dynamic> json,
  Set<String> allowed,
  String label,
) {
  final unknown = json.keys.where((key) => !allowed.contains(key));
  if (unknown.isNotEmpty) {
    throw FormatException('$label contains unknown field ${unknown.first}.');
  }
}
