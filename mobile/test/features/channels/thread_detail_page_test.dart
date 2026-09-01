/// Page-level tests for HW-005: tapping the HW-004 thread task chip opens the
/// read-only task detail sheet.
///
/// Mounts the REAL `ThreadDetailPage` with the same provider-override pattern
/// `channel_detail_page_test.dart` uses (this file's harness is a trimmed copy
/// of that file's `_buildTestable`, plus the tasks-API overrides HW-004/HW-005
/// added), so the tap path exercised here is exactly the production path:
/// page InkWell → `showTaskDetailSheet` → real `TasksApi.getTask` against a
/// `MockClient` serving both the chip lookup and the detail fetch.
library;

import 'dart:convert';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channel_messages_provider.dart';
import 'package:buzz/features/channels/channel_typing_provider.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/mobile_huddle_controller.dart';
import 'package:buzz/features/channels/send_message_provider.dart';
import 'package:buzz/features/channels/thread_detail_page.dart';
import 'package:buzz/features/channels/thread_replies_provider.dart';
import 'package:buzz/features/channels/timeline_message.dart';
import 'package:buzz/features/channels/unread_badge/observed_unread_event.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/tasks/tasks_api.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;
import 'package:shared_preferences/shared_preferences.dart';

const _channelId = '11111111-2222-4333-8444-555555555555';
const _baseUrl = 'https://relay.example.com';

final _testChannel = Channel(
  id: _channelId,
  name: 'general',
  channelType: 'stream',
  visibility: 'open',
  description: 'General discussion',
  createdBy: 'abc123',
  createdAt: DateTime(2025),
  memberCount: 5,
  isMember: true,
);

NostrEvent _textMsg({
  required String id,
  required String pubkey,
  required String content,
  int createdAt = 1000,
}) => NostrEvent(
  id: id,
  pubkey: pubkey,
  createdAt: createdAt,
  kind: EventKind.streamMessage,
  tags: [
    ['h', _channelId],
  ],
  content: content,
  sig: '',
);

// ---------------------------------------------------------------------------
// Fakes (trimmed copies of the ones in channel_detail_page_test.dart)
// ---------------------------------------------------------------------------

class _FakeMessagesNotifier extends ChannelMessagesNotifier {
  _FakeMessagesNotifier(this._messages) : super(_channelId);

  final List<NostrEvent> _messages;

  @override
  AsyncValue<List<NostrEvent>> build() => AsyncData(_messages);

  @override
  bool get hasLoadedMessages => true;

  @override
  bool get reachedOldest => true;

  @override
  Future<bool> fetchOlder() async => false;
}

class _FakeTypingNotifier extends ChannelTypingNotifier {
  _FakeTypingNotifier() : super(_channelId);

  @override
  List<TypingEntry> build() => const [];
}

class _FakeUserCacheNotifier extends UserCacheNotifier {
  _FakeUserCacheNotifier(this._users);

  final Map<String, UserProfile> _users;

  @override
  Map<String, UserProfile> build() => _users;
}

class _FakeProfileNotifier extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async =>
      const UserProfile(pubkey: 'self', displayName: 'Self');
}

class _FakeChannelsNotifier extends ChannelsNotifier {
  _FakeChannelsNotifier(this._channels);

  final List<Channel> _channels;

  @override
  Future<List<Channel>> build() => SynchronousFuture(_channels);

  @override
  Map<String, Map<String, ObservedUnreadEvent>>
  get observedUnreadEventsByChannel => const {};
}

class _FakeThreadLocalRepliesNotifier extends ThreadLocalRepliesNotifier {
  _FakeThreadLocalRepliesNotifier(super.args, this._replies);

  final List<NostrEvent> _replies;

  @override
  List<NostrEvent> build() => _replies;
}

class _FakeRelaySession extends RelaySessionNotifier {
  @override
  SessionState build() =>
      const SessionState(status: SessionStatus.disconnected);
}

class _TestAppLifecycleNotifier extends AppLifecycleNotifier {
  @override
  AppLifecycleState build() => AppLifecycleState.resumed;
}

class _InertReadStateNotifier extends ReadStateNotifier {
  @override
  ReadStateState build() => const ReadStateState.inert();
}

class _StaticRelayConfig extends RelayConfigNotifier {
  _StaticRelayConfig(this._nsec);

  final String? _nsec;

  @override
  RelayConfig build() => RelayConfig(baseUrl: _baseUrl, nsec: _nsec);
}

// ---------------------------------------------------------------------------
// Harness
// ---------------------------------------------------------------------------

/// Shared mock prefs for the compose bar's draft store. Initialized in [main].
late SharedPreferences _testPrefs;

Widget _buildThreadPage({
  required Future<http.Response> Function(http.Request request) taskHandler,
  required String nsec,
}) {
  final root = _textMsg(id: 'root', pubkey: 'alice', content: 'Thread root');
  final timeline = formatTimeline([root]);
  final session = _FakeRelaySession();
  const repliesArgs = ThreadRepliesArgs(channelId: _channelId, rootId: 'root');

  return ProviderScope(
    retry: (_, _) => null,
    overrides: [
      channelMessagesProvider(
        _channelId,
      ).overrideWith(() => _FakeMessagesNotifier([root])),
      channelTypingProvider(
        _channelId,
      ).overrideWith(() => _FakeTypingNotifier()),
      userCacheProvider.overrideWith(
        () => _FakeUserCacheNotifier(const {
          'alice': UserProfile(pubkey: 'alice', displayName: 'Alice'),
        }),
      ),
      profileProvider.overrideWith(() => _FakeProfileNotifier()),
      channelsProvider.overrideWith(
        () => _FakeChannelsNotifier([_testChannel]),
      ),
      channelDetailsProvider(
        _channelId,
      ).overrideWith((ref) async => ChannelDetails.fromChannel(_testChannel)),
      channelMembersProvider(
        _channelId,
      ).overrideWith((ref) async => const <ChannelMember>[]),
      channelBotPubkeysProvider(
        _channelId,
      ).overrideWith((ref) async => const <String>{}),
      agentOwnersProvider.overrideWith((ref) async => const <String, String>{}),
      threadRepliesProvider(
        repliesArgs,
      ).overrideWith((ref) async => <NostrEvent>[]),
      threadLocalRepliesProvider(repliesArgs).overrideWith(
        () => _FakeThreadLocalRepliesNotifier(repliesArgs, const []),
      ),
      relaySessionProvider.overrideWith(() => session),
      relayConfigProvider.overrideWith(() => _StaticRelayConfig(nsec)),
      relayClientProvider.overrideWithValue(
        RelayClient(baseUrl: 'http://localhost:3000'),
      ),
      readStateProvider.overrideWith(() => _InertReadStateNotifier()),
      appLifecycleProvider.overrideWith(_TestAppLifecycleNotifier.new),
      huddleLifecycleProvider(
        _channelId,
      ).overrideWith((ref) async => <NostrEvent>[]),
      sendMessageProvider.overrideWith(
        (ref) => SendMessage(
          signedEventRelay: SignedEventRelay(session: session, nsec: null),
          fetchMembers: (_) async => const [],
          readUserCache: () => const {},
          addLocalMessage: (_, _) {},
          completeLocalMessage: (_, _) {},
          removeLocalMessage: (_, _) {},
        ),
      ),
      tasksHttpClientProvider.overrideWith(
        (ref) => http_testing.MockClient(taskHandler),
      ),
      savedPrefsProvider.overrideWithValue(_testPrefs),
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      home: ThreadDetailPage(
        threadHead: timeline.first,
        allMessages: timeline,
        channelId: _channelId,
        currentPubkey: null,
        isMember: true,
        isArchived: false,
      ),
    ),
  );
}

Map<String, dynamic> _taskJson({
  String id = 'task-1',
  String title = 'Ship the digest contract',
  String status = 'in_progress',
}) => {
  'id': id,
  'title': title,
  'status': status,
  'priority': 0,
  'created_at': 1786000000,
  'updated_at': 1786000060,
};

final _chipTap = find.byKey(const ValueKey('thread-task-chip-tap'));
final _chip = find.byKey(const ValueKey('thread-task-chip'));
final _sheetTitle = find.byKey(const ValueKey('task-detail-title'));

void main() {
  late String nsec;

  setUp(() async {
    SharedPreferences.setMockInitialValues({});
    _testPrefs = await SharedPreferences.getInstance();
    nsec = nostr.Keys.generate().nsec;
  });

  testWidgets('tapping the thread task chip opens the task detail sheet', (
    tester,
  ) async {
    var detailCalls = 0;
    await tester.pumpWidget(
      _buildThreadPage(
        nsec: nsec,
        taskHandler: (request) async {
          final url = request.url;
          if (url.path == '/api/tasks' &&
              url.queryParameters['source_ref'] == 'root') {
            // The chip's reverse lookup: this thread HAS produced a task.
            return http.Response(
              jsonEncode({
                'tasks': [_taskJson()],
              }),
              200,
            );
          }
          if (url.path == '/api/tasks/task-1') {
            detailCalls++;
            return http.Response(
              jsonEncode({
                'task': _taskJson(),
                'events': [
                  {
                    'id': 1,
                    'task_id': 'task-1',
                    'action': 'created',
                    'created_at': 1786000000,
                    'actor': 'a' * 64,
                  },
                  {
                    'id': 2,
                    'task_id': 'task-1',
                    'action': 'commented',
                    'created_at': 1786000005,
                    'body': 'Agent is on it',
                  },
                ],
              }),
              200,
            );
          }
          fail('unexpected task request: ${request.method} $url');
        },
      ),
    );
    await tester.pumpAndSettle();

    // HW-004's chip renders, now wrapped by HW-005's page-level tap target.
    expect(_chip, findsOneWidget);
    expect(_chipTap, findsOneWidget);

    await tester.tap(_chipTap);
    await tester.pumpAndSettle();

    expect(detailCalls, 1);
    expect(_sheetTitle, findsOneWidget);
    expect(tester.widget<Text>(_sheetTitle).data, 'Ship the digest contract');
    expect(find.byKey(const ValueKey('task-event-row-1')), findsOneWidget);
    expect(find.byKey(const ValueKey('task-event-row-2')), findsOneWidget);

    // Dismissing returns to the thread with the chip unchanged.
    final closeButton = find.byWidgetPredicate(
      (widget) => widget is IconButton && widget.tooltip == 'Close sheet',
    );
    expect(closeButton, findsOneWidget);
    await tester.tap(closeButton);
    await tester.pumpAndSettle();

    expect(_sheetTitle, findsNothing);
    expect(_chip, findsOneWidget);
  });

  testWidgets('a thread with no task renders no chip and no tap target', (
    tester,
  ) async {
    await tester.pumpWidget(
      _buildThreadPage(
        nsec: nsec,
        taskHandler: (request) async {
          final url = request.url;
          if (url.path == '/api/tasks' &&
              url.queryParameters['source_ref'] == 'root') {
            return http.Response(jsonEncode({'tasks': []}), 200);
          }
          fail('unexpected task request: ${request.method} $url');
        },
      ),
    );
    await tester.pumpAndSettle();

    expect(_chip, findsNothing);
    expect(_chipTap, findsNothing);
  });

  testWidgets('a failed lookup renders no chip — never a false signal', (
    tester,
  ) async {
    await tester.pumpWidget(
      _buildThreadPage(
        nsec: nsec,
        taskHandler: (request) async =>
            http.Response('{"error":"internal"}', 500),
      ),
    );
    await tester.pumpAndSettle();

    expect(_chip, findsNothing);
    expect(_chipTap, findsNothing);
    expect(tester.takeException(), isNull);
  });
}
