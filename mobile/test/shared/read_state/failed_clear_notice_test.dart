import 'package:buzz/shared/read_state/failed_clear_notice.dart';
import 'package:buzz/shared/read_state/read_state_manager.dart';
import 'package:buzz/shared/read_state/read_state_provider.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

class _ReadState extends ReadStateNotifier {
  @override
  ReadStateState build() => const ReadStateState(
    isReady: true,
    pubkey: 'self',
    contexts: {},
    version: 0,
  );

  void fail(Set<String> channels) => state = ReadStateState(
    isReady: true,
    pubkey: 'self',
    contexts: const {},
    version: state.version + 1,
    failedClears: channels,
  );

  void error(Object? error) => state = ReadStateState(
    isReady: true,
    pubkey: 'self',
    contexts: const {},
    version: state.version + 1,
    syncError: error,
  );
}

void main() {
  const notice = 'Couldn’t sync marking a channel as read. Try again.';

  testWidgets('reports each newly failed clear once, on any screen', (
    tester,
  ) async {
    final readState = _ReadState();
    await tester.pumpWidget(
      ProviderScope(
        overrides: [readStateProvider.overrideWith(() => readState)],
        child: const MaterialApp(
          home: FailedClearNotice(child: Scaffold(body: Text('Inbox'))),
        ),
      ),
    );
    expect(find.text(notice), findsNothing);

    readState.fail({'a'});
    await tester.pump();
    expect(find.text(notice), findsOneWidget);

    ScaffoldMessengerState messenger() =>
        tester.state(find.byType(ScaffoldMessenger));
    messenger().removeCurrentSnackBar();
    await tester.pumpAndSettle();

    // A standing failure, or one that resolves, is not reported again.
    readState.fail({'a'});
    await tester.pumpAndSettle();
    readState.fail({});
    await tester.pumpAndSettle();
    expect(find.text(notice), findsNothing);

    // A new channel failing is.
    readState.fail({'b'});
    await tester.pump();
    expect(find.text(notice), findsOneWidget);
  });

  testWidgets('reports a read refused for capacity once as it starts', (
    tester,
  ) async {
    final readState = _ReadState();
    await tester.pumpWidget(
      ProviderScope(
        overrides: [readStateProvider.overrideWith(() => readState)],
        child: const MaterialApp(
          home: FailedClearNotice(child: Scaffold(body: Text('Inbox'))),
        ),
      ),
    );
    readState.error(StateError('offline'));
    await tester.pump();
    expect(find.text(readStateFullMessage), findsNothing);

    readState.error(StateError(readStateFullMessage));
    await tester.pump();
    expect(find.text(readStateFullMessage), findsOneWidget);
    tester
        .state<ScaffoldMessengerState>(find.byType(ScaffoldMessenger))
        .removeCurrentSnackBar();
    await tester.pumpAndSettle();

    // A standing refusal is not reported again.
    readState.error(StateError(readStateFullMessage));
    await tester.pumpAndSettle();
    expect(find.text(readStateFullMessage), findsNothing);
  });
}
