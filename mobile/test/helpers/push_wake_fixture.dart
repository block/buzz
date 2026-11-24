import 'dart:async';
import 'dart:convert';
import 'dart:io';

import 'package:buzz/shared/community/community.dart';
import 'package:buzz/shared/push/push_subscription.dart';
import 'package:buzz/shared/push/push_wake_resolver.dart';
import 'package:buzz/shared/push/shown_message_store.dart';
import 'package:buzz/shared/relay/nostr_models.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;
import 'package:uuid/uuid.dart';

/// The present, as every part of a wake journey sees it.
final pushWakeClock = DateTime.utc(2026, 11, 15, 12);

/// Communities on fake relays, and a wake that reads them as the headless
/// handler does. Only the network boundary is fake: the relays answer
/// `POST /query` over a [MockClient]. The resolver, its shown-message store
/// (real files in [directory]) and the eligibility rules are the real ones.
class PushWakeFixture {
  PushWakeFixture(this.directory);

  final Directory directory;
  final relays = <FakePushRelay>[];

  late final http.Client client = MockClient(
    (request) => relays
        .firstWhere((relay) => relay.host == request.url.host)
        .handle(request),
  );

  FakePushRelay addCommunity() {
    final relay = FakePushRelay._(relays.length);
    relays.add(relay);
    return relay;
  }

  /// A store as another isolate or process would open it: its own instance
  /// over the same files.
  ShownMessageStore openStore() =>
      DirectoryShownMessageStore(directory, now: () => pushWakeClock);

  Future<PushWakeResult> wake({
    ShownMessageStore? store,
    PushWakeTimeouts timeouts = const PushWakeTimeouts(),
    Future<List<Community>> Function()? loadCommunities,
  }) => resolvePushWake(
    store: store ?? openStore(),
    loadCommunities:
        loadCommunities ?? () async => [for (final r in relays) r.community],
    httpClient: client,
    timeouts: timeouts,
    now: () => pushWakeClock,
  );
}

class FakeChannel {
  FakeChannel(this.id, this.name, {required this.isDm});
  final String id;
  final String name;
  final bool isDm;
}

class FakePerson {
  FakePerson(this.name) : keys = nostr.Keys.generate();
  final String name;
  final nostr.Keys keys;
  String get pubkey => keys.public;
}

class FakePushRelay {
  FakePushRelay._(this.index) : me = FakePerson('Me');

  final int index;
  final FakePerson me;
  final events = <NostrEvent>[];
  final queries = <List<Map<String, dynamic>>>[];
  final _channels = <FakeChannel>[];
  final _muted = <String>{};
  var _sequence = 0;

  /// Withholds answers until completed, as a slow relay would.
  Completer<void>? hold;

  /// Never answers.
  bool silent = false;

  /// Refuses connections.
  bool unreachable = false;

  /// Refuses connections once it has answered this many queries.
  int? unreachableAfter;

  /// Answers every query with this body instead of the stored events, as a
  /// hostile relay would.
  http.Response Function()? hostileResponse;

  /// Never answers queries for profiles, as a metadata lookup slower than
  /// any budget.
  bool slowProfiles = false;

  String get host => 'relay-$index.example.org';
  String get communityId => 'community-$index';

  Community get community => Community(
    id: communityId,
    name: 'Community $index',
    relayUrl: 'https://$host',
    pubkey: me.pubkey,
    nsec: me.keys.nsec,
    pushSubscriptionState: BuzzPushLeaseSubscriptionState.desired(
      desired: buildDesiredBuzzPushSubscriptions(
        myPubkey: me.pubkey,
        channelIds: [
          for (final channel in _channels)
            if (channel.isDm) channel.id,
        ],
        mutedChannelIds: _muted,
      ),
    ),
    addedAt: pushWakeClock,
  );

  FakeChannel dmChannel() => _addChannel(isDm: true);
  FakeChannel channel({bool forged = false}) =>
      _addChannel(isDm: false, forged: forged);

  FakeChannel _addChannel({required bool isDm, bool forged = false}) {
    final id = const Uuid().v4();
    final channel = FakeChannel(id, 'room-${id.substring(0, 4)}', isDm: isDm);
    _channels.add(channel);
    final metadata = _sign(
      kind: 39000,
      tags: [
        ['d', id],
        ['name', channel.name],
      ],
      content: '',
      author: nostr.Keys.generate(),
    );
    events.add(forged ? _forge(metadata) : metadata);
    return channel;
  }

  void mute(FakeChannel channel) => _muted.add(channel.id);

  /// A person with a profile. A [forged] profile carries a signature that
  /// isn't its author's, as an untrusted relay could serve.
  FakePerson person(String name, {bool forged = false}) {
    final person = FakePerson(name);
    final profile = _sign(
      kind: 0,
      tags: const [],
      content: jsonEncode({'display_name': name}),
      author: person.keys,
    );
    events.add(forged ? _forge(profile) : profile);
    return person;
  }

  NostrEvent _forge(NostrEvent event) => NostrEvent.fromJson({
    ...event.toJson(),
    'sig': nostr.Event.from(
      kind: 1,
      tags: const [],
      content: 'someone else',
      secretKey: nostr.Keys.generate().secret,
      verify: false,
    ).sig,
  });

  /// Stores a message that arrived `age` before now (default: each message a
  /// second after the previous one, ending a few minutes ago).
  NostrEvent message(
    FakeChannel channel,
    FakePerson author,
    String content, {
    FakePerson? mention,
    List<String> extraMentions = const [],
    List<List<String>> extraTags = const [],
    Duration? age,
    DateTime? at,
  }) {
    final createdAt =
        at ??
        pushWakeClock
            .subtract(age ?? const Duration(minutes: 5))
            .add(Duration(seconds: _sequence++));
    final event = _sign(
      kind: EventKind.streamMessage,
      tags: [
        ['h', channel.id],
        if (mention != null) ['p', mention.pubkey],
        for (final pubkey in extraMentions) ['p', pubkey],
        ...extraTags,
      ],
      content: content,
      author: author.keys,
      createdAt: createdAt.millisecondsSinceEpoch ~/ 1000,
    );
    events.add(event);
    return event;
  }

  NostrEvent _sign({
    required int kind,
    required List<List<String>> tags,
    required String content,
    required nostr.Keys author,
    int? createdAt,
  }) => NostrEvent.fromJson(
    nostr.Event.from(
      kind: kind,
      tags: tags,
      content: content,
      createdAt: createdAt,
      secretKey: author.secret,
      verify: false,
    ).toMap(),
  );

  Future<http.Response> handle(http.Request request) async {
    if (unreachable || queries.length >= (unreachableAfter ?? 1 << 30)) {
      throw const SocketException('connection refused');
    }
    if (request.method != 'POST' ||
        request.url.path != '/query' ||
        request.headers['Authorization'] == null) {
      return http.Response('unanswerable', 501);
    }
    final filters = (jsonDecode(request.body) as List)
        .cast<Map<String, dynamic>>();
    queries.add(filters);
    if (silent) await Completer<void>().future;
    await hold?.future;
    final hostile = hostileResponse;
    if (hostile != null) return hostile();
    if (slowProfiles &&
        filters.any((f) => (f['kinds'] as List?)?.contains(0) ?? false)) {
      await Completer<void>().future;
    }
    final result = <String, NostrEvent>{};
    for (final filter in filters) {
      final matches = events.where((e) => _matches(filter, e)).toList()
        ..sort(
          (a, b) => a.createdAt != b.createdAt
              ? b.createdAt.compareTo(a.createdAt)
              : a.id.compareTo(b.id),
        );
      for (final event in matches.take(filter['limit'] as int)) {
        result[event.id] = event;
      }
    }
    return http.Response(
      jsonEncode([for (final e in result.values) e.toJson()]),
      200,
      headers: {'content-type': 'application/json'},
    );
  }

  bool _matches(Map<String, dynamic> filter, NostrEvent event) {
    bool allows(String key, Object value) =>
        (filter[key] as List?)?.contains(value) ?? true;
    if (!allows('authors', event.pubkey) || !allows('kinds', event.kind)) {
      return false;
    }
    final since = filter['since'] as int?;
    if (since != null && event.createdAt < since) return false;
    final until = filter['until'] as int?;
    if (until != null && event.createdAt > until) return false;
    for (final MapEntry(:key, :value) in filter.entries) {
      if (!key.startsWith('#')) continue;
      final wanted = (value as List).cast<String>();
      if (!event.tags.any(
        (tag) =>
            tag.length > 1 &&
            tag[0] == key.substring(1) &&
            wanted.contains(tag[1]),
      )) {
        return false;
      }
    }
    return true;
  }
}

/// A store whose claims take [delay], as slow storage would, and whose
/// [newestShown] waits for [newestGate].
class SlowShownMessageStore implements ShownMessageStore {
  SlowShownMessageStore(this._inner, this.delay, {this.newestGate});

  final ShownMessageStore _inner;
  final Duration delay;
  final Future<void>? newestGate;

  @override
  Future<DateTime?> newestShown(String community) async {
    await newestGate;
    return _inner.newestShown(community);
  }

  @override
  Future<bool> claim(
    String community,
    String eventId,
    DateTime createdAt,
  ) async {
    await Future<void>.delayed(delay);
    return _inner.claim(community, eventId, createdAt);
  }
}

/// A store whose claims for [community] fail after [succeed] of them, as when
/// storage fills midway through a wake.
class FailingShownMessageStore implements ShownMessageStore {
  FailingShownMessageStore(this._inner, this.community, {this.succeed = 0});

  final ShownMessageStore _inner;
  final String community;
  int succeed;

  @override
  Future<DateTime?> newestShown(String community) =>
      _inner.newestShown(community);

  @override
  Future<bool> claim(String community, String eventId, DateTime createdAt) {
    if (community == this.community && succeed-- <= 0) {
      throw const FileSystemException('No space left on device');
    }
    return _inner.claim(community, eventId, createdAt);
  }
}

/// A store whose claims for [community] never answer after [succeed] of them,
/// as storage that hangs midway through a wake.
class StalledShownMessageStore implements ShownMessageStore {
  StalledShownMessageStore(this._inner, this.community, {this.succeed = 0});

  final ShownMessageStore _inner;
  final String community;
  int succeed;

  @override
  Future<DateTime?> newestShown(String community) =>
      _inner.newestShown(community);

  @override
  Future<bool> claim(String community, String eventId, DateTime createdAt) {
    if (community == this.community && succeed-- <= 0) {
      return Completer<bool>().future;
    }
    return _inner.claim(community, eventId, createdAt);
  }
}
