import 'package:buzz/features/channels/me/me_workspace_picker.dart';
import 'package:buzz/shared/me/me_groups.dart';
import 'package:buzz/shared/me/me_sync.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

class _Sync extends MeSync {
  _Sync()
    : super(
        RelaySessionNotifier(),
        const RelayConfig(baseUrl: 'https://relay.test'),
      );
  MeGroups groups = const MeGroups(
    groups: [
      {'id': 'work', 'name': 'Work', 'defaultTemplateId': ''},
    ],
    channels: ['chat', 'other'],
    assignments: {'other': 'work'},
  );
  @override
  Future<void> update(MeGroups Function(MeGroups) change) async {
    groups = change(groups);
  }
}

class _Header extends HookConsumerWidget {
  const _Header();
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    useState(0);
    return useMeWorkspacePicker(
      context,
      ref,
      channelId: 'chat',
      enabled: true,
    ).action;
  }
}

void main() {
  testWidgets(
    'workspace menu moves a chat into a workspace and back to No Workspace',
    (tester) async {
      final sync = _Sync();
      await tester.pumpWidget(
        ProviderScope(
          overrides: [
            meSyncProvider.overrideWithValue(sync),
            meGroupsProvider.overrideWith((ref) => Stream.value(sync.groups)),
          ],
          child: MaterialApp(
            theme: AppTheme.light(),
            home: const Scaffold(body: _Header()),
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.byType(PopupMenuButton<String>));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Work'));
      await tester.pumpAndSettle();
      expect(sync.groups.assignments['chat'], 'work');
      await tester.tap(find.byType(PopupMenuButton<String>));
      await tester.pumpAndSettle();
      await tester.tap(find.text('No Workspace'));
      await tester.pumpAndSettle();
      expect(sync.groups.assignments.containsKey('chat'), isFalse);
      expect(sync.groups.assignments['other'], 'work');
      expect(sync.groups.channels, ['chat', 'other']);
    },
  );
}
