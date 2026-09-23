import 'dart:convert';

import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/tasks/create_task_sheet.dart';
import 'package:buzz/shared/tasks/tasks_api.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:http/http.dart' as http;
import 'package:http/testing.dart' as http_testing;
import 'package:nostr/nostr.dart' as nostr;

const _channelId = 'channel-1';
const _agentPubkey =
    'aa11bb22cc33dd44ee55ff6600112233445566778899aabbccddeeff0';

final _titleField = find.byKey(const ValueKey('create-task-title'));
final _bodyField = find.byKey(const ValueKey('create-task-body'));
final _submit = find.byKey(const ValueKey('create-task-submit'));

late String _nsec;

class _TestRelayConfig extends RelayConfigNotifier {
  _TestRelayConfig({required this.nsec});

  final String? nsec;

  @override
  RelayConfig build() =>
      RelayConfig(baseUrl: 'https://relay.example.com', nsec: nsec);
}

class _FakeUserCache extends UserCacheNotifier {
  _FakeUserCache(this.profiles);

  final Map<String, UserProfile> profiles;

  @override
  Map<String, UserProfile> build() => profiles;
}

/// A page with one button that opens the sheet, mirroring how a composer or a
/// message action would invoke it.
class _Harness extends ConsumerWidget {
  const _Harness({required this.channelName});

  final String channelName;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    return Scaffold(
      body: Center(
        child: TextButton(
          onPressed: () => showCreateTaskSheet(
            context: context,
            ref: ref,
            channelId: _channelId,
            channelName: channelName,
            sourceEventId: 'event-1',
          ),
          child: const Text('open'),
        ),
      ),
    );
  }
}

Widget _app({
  required http.Client client,
  String? nsec,
  Set<String> agentPubkeys = const {},
  Map<String, String> directoryNames = const {},
  Map<String, UserProfile> profiles = const {},
  String channelName = 'general',
}) {
  return ProviderScope(
    overrides: [
      relayConfigProvider.overrideWith(
        () => _TestRelayConfig(nsec: nsec ?? _nsec),
      ),
      tasksHttpClientProvider.overrideWithValue(client),
      userCacheProvider.overrideWith(() => _FakeUserCache(profiles)),
      channelBotPubkeysProvider(
        _channelId,
      ).overrideWith((ref) async => agentPubkeys),
      agentDirectoryDisplayNamesProvider.overrideWithValue(directoryNames),
    ],
    child: MaterialApp(
      theme: AppTheme.light(),
      home: _Harness(channelName: channelName),
    ),
  );
}

Future<void> _openSheet(WidgetTester tester) async {
  await tester.tap(find.text('open'));
  await tester.pumpAndSettle();
}

/// Taps a row inside the sheet's scrolling field area.
///
/// The form is taller than the 600px test viewport, so a row below the fold is
/// built but off-screen; scrolling it into view first is what a user does too.
Future<void> _tapRow(WidgetTester tester, Finder finder) async {
  await tester.ensureVisible(finder);
  await tester.pumpAndSettle();
  await tester.tap(finder);
  await tester.pump();
}

void main() {
  setUp(() => _nsec = nostr.Keys.generate().nsec);

  testWidgets('requires a title before it will submit', (tester) async {
    final client = http_testing.MockClient(
      (request) async => http.Response('{}', 200),
    );
    await tester.pumpWidget(_app(client: client));
    await _openSheet(tester);

    expect(find.text('New task'), findsOneWidget);
    expect(tester.widget<FilledButton>(_submit).onPressed, isNull);

    await tester.enterText(_titleField, 'Ship the relay change');
    await tester.pump();

    expect(tester.widget<FilledButton>(_submit).onPressed, isNotNull);
  });

  testWidgets('posts the typed task and confirms it', (tester) async {
    late http.Request captured;
    final client = http_testing.MockClient((request) async {
      captured = request;
      return http.Response(
        jsonEncode({
          'id': 'task-1',
          'title': 'Ship the relay change',
          'status': 'todo',
          'priority': 0,
          'created_at': 1786000000,
          'updated_at': 1786000000,
        }),
        200,
      );
    });

    await tester.pumpWidget(_app(client: client));
    await _openSheet(tester);
    await tester.enterText(_titleField, 'Ship the relay change');
    await tester.enterText(_bodyField, 'behind a flag');
    await tester.pump();
    await tester.tap(_submit);
    await tester.pumpAndSettle();

    expect(captured.url.path, '/api/tasks');
    expect(jsonDecode(captured.body), {
      'title': 'Ship the relay change',
      'body': 'behind a flag',
      'channel_id': _channelId,
      'source_ref': 'event-1',
      'source': 'mobile',
    });
    // The sheet closes and the confirmation lands on the page beneath it.
    expect(_titleField, findsNothing);
    expect(find.text('Task created'), findsOneWidget);
  });

  testWidgets('scoping to the whole community drops channel_id', (
    tester,
  ) async {
    late http.Request captured;
    final client = http_testing.MockClient((request) async {
      captured = request;
      return http.Response(
        jsonEncode({
          'id': 'task-1',
          'title': 'Community wide',
          'status': 'todo',
          'priority': 0,
          'created_at': 1786000000,
          'updated_at': 1786000000,
        }),
        200,
      );
    });

    await tester.pumpWidget(_app(client: client));
    await _openSheet(tester);
    await tester.enterText(_titleField, 'Community wide');
    await tester.pump();

    expect(find.text('#general'), findsOneWidget);
    await _tapRow(
      tester,
      find.byKey(const ValueKey('create-task-scope-community')),
    );
    await tester.tap(_submit);
    await tester.pumpAndSettle();

    expect(
      (jsonDecode(captured.body) as Map).containsKey('channel_id'),
      isFalse,
    );
    expect((jsonDecode(captured.body) as Map)['title'], 'Community wide');
  });

  testWidgets('mentioning channel agents prefixes the body', (tester) async {
    late http.Request captured;
    final client = http_testing.MockClient((request) async {
      captured = request;
      return http.Response(
        jsonEncode({
          'id': 'task-1',
          'title': 'Investigate the drop',
          'status': 'todo',
          'priority': 0,
          'created_at': 1786000000,
          'updated_at': 1786000000,
        }),
        200,
      );
    });

    await tester.pumpWidget(
      _app(
        client: client,
        agentPubkeys: const {_agentPubkey},
        directoryNames: const {_agentPubkey: 'Ada'},
      ),
    );
    await _openSheet(tester);
    await tester.enterText(_titleField, 'Investigate the drop');
    await tester.enterText(_bodyField, 'starts around 03:00');
    await tester.pump();

    final agentRow = find.byKey(const ValueKey('create-task-assign-agents'));
    expect(agentRow, findsOneWidget);
    await _tapRow(tester, agentRow);
    await tester.tap(_submit);
    await tester.pumpAndSettle();

    expect(
      (jsonDecode(captured.body) as Map)['body'],
      '@Ada\n\nstarts around 03:00',
    );
  });

  testWidgets('hides the agent row when the channel has no agents', (
    tester,
  ) async {
    final client = http_testing.MockClient(
      (request) async => http.Response('{}', 200),
    );
    await tester.pumpWidget(_app(client: client));
    await _openSheet(tester);

    expect(
      find.byKey(const ValueKey('create-task-assign-agents')),
      findsNothing,
    );
  });

  testWidgets('shows the relay message and keeps the sheet open on failure', (
    tester,
  ) async {
    final client = http_testing.MockClient(
      (request) async => http.Response(
        jsonEncode(const {'error': 'title must be at most 200 characters'}),
        400,
      ),
    );

    await tester.pumpWidget(_app(client: client));
    await _openSheet(tester);
    await tester.enterText(_titleField, 'Ship it');
    await tester.pump();
    await tester.tap(_submit);
    await tester.pumpAndSettle();

    expect(find.text('title must be at most 200 characters'), findsOneWidget);
    // The sheet must survive so the typed content is not lost.
    expect(_titleField, findsOneWidget);
    expect(find.text('Task created'), findsNothing);
  });

  testWidgets('refuses to open without a signing key', (tester) async {
    final client = http_testing.MockClient(
      (request) async => http.Response('{}', 200),
    );
    await tester.pumpWidget(_app(client: client, nsec: ''));
    await _openSheet(tester);

    expect(find.text('Sign in to create tasks'), findsOneWidget);
    expect(find.text('New task'), findsNothing);
  });

  testWidgets('labels the scope row when the channel has no name', (
    tester,
  ) async {
    final client = http_testing.MockClient(
      (request) async => http.Response('{}', 200),
    );
    await tester.pumpWidget(_app(client: client, channelName: ''));
    await _openSheet(tester);

    expect(find.text('This conversation'), findsOneWidget);
  });
}
