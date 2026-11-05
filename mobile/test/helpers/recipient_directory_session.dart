import 'dart:convert';

import 'package:buzz/shared/relay/relay.dart';

/// A connected relay whose profile search finds [people] by name, as the
/// New DM and Add members pickers search. Keys in [agents] are relay
/// directory agents (kind:10100) that answer anyone.
class RecipientDirectorySession extends RelaySessionNotifier {
  RecipientDirectorySession(
    this.people, {
    this.agents = const {},
    this.pictures = const {},
  });

  /// Display names by public key.
  final Map<String, String> people;
  final Set<String> agents;

  /// Profile picture URLs by public key.
  final Map<String, String> pictures;
  final searches = <String>[];

  @override
  SessionState build() => const SessionState(status: SessionStatus.connected);

  @override
  Future<List<NostrEvent>> searchProfiles(
    String query, {
    int limit = 50,
    Duration timeout = const Duration(seconds: 8),
  }) async {
    searches.add(query);
    final needle = query.trim().toLowerCase();
    return [
      for (final MapEntry(:key, :value) in people.entries)
        if (value.toLowerCase().contains(needle))
          NostrEvent(
            id: '$key-profile',
            pubkey: key,
            createdAt: 1700000000,
            kind: 0,
            tags: const [],
            content: jsonEncode({
              'display_name': value,
              'picture': ?pictures[key],
            }),
            sig: 'sig',
          ),
    ];
  }

  @override
  Future<List<NostrEvent>> queryRelay(
    List<NostrFilter> filters, {
    Duration timeout = const Duration(seconds: 8),
  }) async => const [];

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [
    if (filter.kinds.contains(10100))
      for (final key in agents)
        NostrEvent(
          id: '$key-agent',
          pubkey: key,
          createdAt: 1700000000,
          kind: 10100,
          tags: const [],
          content: jsonEncode({
            'display_name': people[key],
            'respond_to': 'anyone',
          }),
          sig: 'sig',
        ),
  ];

  @override
  Future<void Function()> subscribeWithStatus(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
    required void Function(RelaySubscriptionStatus) onStatusChanged,
  }) => subscribe(filter, onEvent, onClosed: onClosed);

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async => () {};
}
