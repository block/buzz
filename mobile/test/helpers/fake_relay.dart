import 'dart:async';
import 'dart:convert';

import 'package:buzz/shared/auth/auth_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart';
import 'package:nostr/nostr.dart' as nostr;

/// An in-memory relay behind the real [RelaySessionNotifier].
///
/// Only the process boundary is fake: the websocket (through the session's
/// socket factory) and the HTTP bridge (through its HTTP client). The session
/// still connects from `build()`, matches REQ/EOSE/OK/CLOSED, batches events,
/// backs off and replays live subscriptions. Replies arrive on microtasks, so
/// tests drive timers with `fakeAsync` or `tester.pump`, never real sleeps.
///
/// It is not a relay emulator: authorization, NIP-42 and JSON framing stay
/// with the relay and socket tests. A request it cannot answer faithfully is
/// recorded and fails the test at teardown, because app code often catches
/// relay errors and falls back.
class FakeRelay {
  /// [now] is the session's wall clock, so a test can model OS suspension:
  /// time passes while no timer runs.
  FakeRelay({DateTime Function()? now}) : _now = now {
    addTearDown(() {
      expect(unanswerable, isEmpty, reason: 'requests FakeRelay cannot answer');
    });
  }

  final keys = nostr.Keys.generate();
  final stored = <NostrEvent>[];
  final published = <NostrEvent>[];
  final reqs = <List<dynamic>>[];
  final queries = <List<Map<String, dynamic>>>[];
  final sockets = <FakeRelaySocket>[];
  final unanswerable = <String>[];
  final _rejections = <({bool Function(NostrEvent) match, String message})>[];
  Completer<void>? _okHold;
  Completer<void>? _echoHold;
  Completer<void>? _reqHold;
  Completer<void>? _authHold;
  final DateTime Function()? _now;

  String get pubkey => keys.public;

  late final session = RelaySessionNotifier(
    socketFactory: _openSocket,
    httpClient: MockClient(_handleHttp),
    now: _now,
  );

  List<Override> get overrides => [
    relaySessionProvider.overrideWith(() => session),
    relayConfigProvider.overrideWith(() => _FixedRelayConfig(keys.nsec)),
    authProvider.overrideWith(_SignedInAuth.new),
  ];

  /// Stores history without notifying live subscriptions.
  void seed(Iterable<NostrEvent> events) => stored.addAll(events);

  /// Another client published [event]: store it and fan it out.
  void deliver(NostrEvent event) {
    stored.add(event);
    _fanOut(event);
  }

  /// Signs an event as [author] (default: a fresh member, not the app user).
  NostrEvent sign({
    required int kind,
    required List<List<String>> tags,
    String content = '',
    int? createdAt,
    nostr.Keys? author,
  }) {
    final event = nostr.Event.from(
      kind: kind,
      tags: tags,
      content: content,
      createdAt: createdAt,
      secretKey: (author ?? nostr.Keys.generate()).secret,
      verify: false,
    );
    return NostrEvent.fromJson(event.toMap());
  }

  /// The next published event that [match]es is answered `OK false` and is
  /// neither stored nor fanned out.
  void rejectNext(String message, {bool Function(NostrEvent)? match}) =>
      _rejections.add((match: match ?? (_) => true, message: message));

  /// Withholds OK for accepted events until the completer completes.
  Completer<void> holdOk() => _okHold = Completer<void>();

  /// Withholds live fan-out of accepted events until the completer completes.
  /// The relay dispatches after commit, so echo and OK have no fixed order.
  Completer<void> holdEcho() => _echoHold = Completer<void>();

  /// Holds new REQs until the completer completes, as a relay's access checks
  /// would: they read stored events, start live fan-out and reach EOSE only
  /// once admitted. Events committed meanwhile reach them only through replay.
  Completer<void> holdReq() => _reqHold = Completer<void>();

  /// Keeps the next connection unauthenticated until the completer completes.
  /// Like the relay, it answers a REQ sent before AUTH with CLOSED.
  Completer<void> holdAuth() => _authHold = Completer<void>();

  /// REQs the relay rejected because they arrived before AUTH.
  final reqsBeforeAuth = <List<dynamic>>[];

  /// Filters of the live subscriptions open on the current connection.
  Iterable<List<Map<String, dynamic>>> get liveFilters =>
      sockets.isEmpty ? const [] : sockets.last._live.values;

  /// Drops the current connection, as a network loss would. Its live
  /// subscriptions and queued replies die with it; stored history survives.
  void drop([Object? error]) => sockets.last.drop(error);

  RelaySocket _openSocket({
    required String wsUrl,
    required String? nsec,
    required void Function(List<dynamic> message) onMessage,
    required void Function() onConnected,
    required void Function(Object? error) onDisconnected,
  }) {
    final socket = FakeRelaySocket._(
      this,
      onMessage: onMessage,
      onConnected: onConnected,
      onDisconnected: onDisconnected,
    );
    sockets.add(socket);
    return socket;
  }

  void _receive(FakeRelaySocket socket, List<dynamic> message) {
    switch (message.first) {
      case 'REQ':
        final subId = message[1] as String;
        if (!socket._authenticated) {
          reqsBeforeAuth.add(message);
          scheduleMicrotask(
            () => socket._reply([
              'CLOSED',
              subId,
              'auth-required: not authenticated',
            ]),
          );
          return;
        }
        reqs.add(message);
        final filters = message.skip(2).cast<Map<String, dynamic>>().toList();
        void admit() {
          if (!socket._authenticated) return;
          final history = _query(filters);
          if (history == null) return;
          socket._live[subId] = filters;
          scheduleMicrotask(() {
            for (final event in history) {
              socket._reply(['EVENT', subId, event.toJson()]);
            }
            socket._reply(['EOSE', subId]);
          });
        }

        final hold = _reqHold;
        hold == null ? admit() : hold.future.then((_) => admit());
      case 'CLOSE':
        socket._live.remove(message[1]);
      case 'EVENT':
        _publish(
          socket,
          NostrEvent.fromJson(message[1] as Map<String, dynamic>),
        );
      default:
        unanswerable.add('${message.first} frame');
    }
  }

  void _publish(FakeRelaySocket socket, NostrEvent event) {
    published.add(event);
    final index = _rejections.indexWhere((r) => r.match(event));
    if (index >= 0) {
      final message = _rejections.removeAt(index).message;
      scheduleMicrotask(() => socket._reply(['OK', event.id, false, message]));
      return;
    }
    stored.add(event);
    final okHold = _okHold?.future;
    final echoHold = _echoHold?.future;
    Future<void> ok() async {
      await okHold;
      socket._reply(['OK', event.id, true, '']);
    }

    Future<void> echo() async {
      await echoHold;
      _fanOut(event);
    }

    scheduleMicrotask(ok);
    scheduleMicrotask(echo);
  }

  void _fanOut(NostrEvent event) {
    for (final socket in sockets) {
      for (final MapEntry(key: subId, value: filters) in socket._live.entries) {
        if (filters.any((f) => _matches(f, event, live: true))) {
          socket._reply(['EVENT', subId, event.toJson()]);
        }
      }
    }
  }

  /// Stored events for [filters], or null after recording why it cannot
  /// answer. Each filter applies its own limit before the union, as the
  /// relay does.
  List<NostrEvent>? _query(List<Map<String, dynamic>> filters) {
    if (!_answerable(filters)) return null;
    final result = <String, NostrEvent>{};
    for (final filter in filters) {
      final limit = filter['limit'] as int?;
      final matches = stored.where((e) => _matches(filter, e)).toList()
        ..sort(_newestFirst);
      for (final event in limit == null ? matches : matches.take(limit)) {
        result[event.id] = event;
      }
    }
    return result.values.toList()..sort(_newestFirst);
  }

  /// Whether [_matches] implements every key in [filters]; records why not.
  /// Checked before matching, so an empty store cannot hide it.
  bool _answerable(List<Map<String, dynamic>> filters) {
    final unknown = {
      for (final filter in filters)
        ...filter.keys.where(
          (key) => !_filterKeys.contains(key) && !key.startsWith('#'),
        ),
    };
    if (unknown.isEmpty) return true;
    unanswerable.add('filter keys ${unknown.toList()}');
    return false;
  }

  /// NIP-01 matching for a filter [_answerable] accepted.
  bool _matches(
    Map<String, dynamic> filter,
    NostrEvent event, {
    bool live = false,
  }) {
    if (!live && filter['limit'] == 0) return false;
    bool allows(String key, Object value) =>
        (filter[key] as List?)?.contains(value) ?? true;
    if (!allows('ids', event.id) ||
        !allows('authors', event.pubkey) ||
        !allows('kinds', event.kind)) {
      return false;
    }
    final since = filter['since'] as int?;
    final until = filter['until'] as int?;
    if (since != null && event.createdAt < since) return false;
    if (until != null && event.createdAt > until) return false;
    // Buzz pagination: strictly after the cursor in `created_at DESC, id ASC`.
    final beforeId = filter['before_id'] as String?;
    if (beforeId != null &&
        event.createdAt == until &&
        event.id.compareTo(beforeId) <= 0) {
      return false;
    }
    for (final MapEntry(:key, :value) in filter.entries) {
      if (!key.startsWith('#')) continue;
      final name = key.substring(1);
      final wanted = (value as List).cast<String>();
      if (!event.tags.any(
        (tag) => tag.length > 1 && tag[0] == name && wanted.contains(tag[1]),
      )) {
        return false;
      }
    }
    return true;
  }

  Future<http.Response> _handleHttp(http.Request request) async {
    if (request.method != 'POST' ||
        request.url.path != '/query' ||
        request.headers['Authorization'] == null) {
      unanswerable.add('${request.method} ${request.url.path}');
      return http.Response('unanswerable', 501);
    }
    final filters = (jsonDecode(request.body) as List)
        .cast<Map<String, dynamic>>();
    queries.add(filters);
    final events = filters.length == 1 && filters.single['top_level'] == true
        ? _channelWindow(filters.single)
        : _query(filters);
    if (events == null) return http.Response('unanswerable', 501);
    return http.Response(jsonEncode([for (final e in events) e.toJson()]), 200);
  }

  /// The NIP-CW top-level channel window: one page of top-level rows plus its
  /// kind:39006 bounds. Thread summaries and aux events are not synthesized
  /// yet; a journey that needs them adds them here.
  List<NostrEvent>? _channelWindow(Map<String, dynamic> filter) {
    final channel = ((filter['#h'] as List).single as String).toLowerCase();
    final beforeId = filter['before_id'] as String?;
    final until = filter['until'] as int?;
    final limit = filter['limit'] as int;
    final base = {
      for (final MapEntry(:key, :value) in filter.entries)
        if (!_windowKeys.contains(key)) key: value,
    };
    if (!_answerable([base])) return null;
    final rows = <NostrEvent>[];
    for (final event in stored) {
      if (!_matches(base, event) || event.threadReference.parentId != null) {
        continue;
      }
      // Rows strictly after the cursor in `created_at DESC, id ASC` order.
      if (until != null &&
          beforeId != null &&
          !(event.createdAt < until ||
              (event.createdAt == until && event.id.compareTo(beforeId) > 0))) {
        continue;
      }
      rows.add(event);
    }
    rows.sort(_newestFirst);
    final page = rows.take(limit).toList();
    final hasMore = rows.length > limit;
    final cursor = until == null || beforeId == null
        ? 'head'
        : '$until:${beforeId.toLowerCase()}';
    return [
      ...page,
      NostrEvent(
        id: 'bounds-$channel-$cursor',
        pubkey: 'relay',
        createdAt: 0,
        kind: EventKind.channelWindowBounds,
        tags: [
          ['d', '$channel:$cursor'],
        ],
        content: jsonEncode({
          'has_more': hasMore,
          'next_cursor': hasMore
              ? {'created_at': page.last.createdAt, 'id': page.last.id}
              : null,
        }),
        sig: 'sig',
      ),
    ];
  }
}

/// The socket the session talks to; [FakeRelay] owns both ends.
class FakeRelaySocket extends RelaySocket {
  FakeRelaySocket._(
    this._relay, {
    required super.onMessage,
    required super.onConnected,
    required super.onDisconnected,
  }) : _onMessage = onMessage,
       _onConnected = onConnected,
       _onDisconnected = onDisconnected,
       super(wsUrl: 'wss://relay.test', nsec: null);

  final FakeRelay _relay;
  final void Function(List<dynamic> message) _onMessage;
  final void Function() _onConnected;
  final void Function(Object? error) _onDisconnected;
  final _live = <String, List<Map<String, dynamic>>>{};
  bool _open = false;
  bool _authenticated = false;

  @override
  Future<void> connect() async {
    _open = true;
    final hold = _relay._authHold;
    _relay._authHold = null;
    if (hold != null) await hold.future;
    if (!_open) return;
    _authenticated = true;
    _onConnected();
  }

  @override
  void send(List<dynamic> payload) {
    if (_open) _relay._receive(this, payload);
  }

  @override
  Future<void> disconnect() async => dispose();

  @override
  void dispose() {
    _open = false;
    _authenticated = false;
    _live.clear();
  }

  void drop(Object? error) {
    if (!_open) return;
    dispose();
    _onDisconnected(error);
  }

  void _reply(List<dynamic> message) {
    if (_open) _onMessage(message);
  }
}

const _filterKeys = {
  'ids',
  'authors',
  'kinds',
  'since',
  'until',
  'limit',
  'before_id',
};
const _windowKeys = {
  'top_level',
  'include_summaries',
  'include_aux',
  'before_id',
  'until',
  'limit',
};

int _newestFirst(NostrEvent a, NostrEvent b) => a.createdAt != b.createdAt
    ? b.createdAt.compareTo(a.createdAt)
    : a.id.compareTo(b.id);

class _FixedRelayConfig extends RelayConfigNotifier {
  _FixedRelayConfig(this._nsec);

  final String _nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.test', nsec: _nsec);
}

class _SignedInAuth extends AuthNotifier {
  @override
  Future<AuthState> build() async =>
      const AuthState(status: AuthStatus.authenticated);
}
