import 'dart:async';

import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_actions_sheet.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_provider.dart';
import 'package:buzz/features/channels/channel_sections/channel_sections_storage.dart';
import 'package:buzz/features/channels/manage_channel_sheet.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

const _currentPubkey = 'me';

Channel _channel({
  String type = 'stream',
  bool isArchived = false,
  bool isMember = true,
}) => Channel(
  id: 'channel-id',
  name: type == 'dm' ? 'Alice' : 'general',
  channelType: type,
  visibility: 'open',
  description: '',
  createdBy: 'owner',
  createdAt: DateTime(2025),
  memberCount: 2,
  isMember: isMember,
  archivedAt: isArchived ? DateTime(2025, 1, 2) : null,
);

Widget _app({
  required Channel channel,
  required Future<List<ChannelMember>> Function() loadMembers,
  bool isUnread = false,
  AsyncValue<Map<String, String>> agentOwners = const AsyncValue.data(
    <String, String>{},
  ),
  ChannelActions Function(Ref ref)? createChannelActions,
  String? currentPubkey = _currentPubkey,
}) => ProviderScope(
  overrides: [
    currentPubkeyProvider.overrideWith((ref) => currentPubkey),
    channelMembersProvider(channel.id).overrideWith((ref) => loadMembers()),
    agentOwnersProvider.overrideWithValue(agentOwners),
    if (createChannelActions != null)
      channelActionsProvider.overrideWith(createChannelActions),
  ],
  child: MaterialApp(
    theme: AppTheme.light(),
    home: Scaffold(
      body: ChannelActionsSheet(channel: channel, isUnread: isUnread),
    ),
  ),
);

Widget _modalApp({
  required Channel channel,
  required Future<List<ChannelMember>> Function() loadMembers,
  required ChannelActions Function(Ref ref) createChannelActions,
  String? canvasContent,
  Future<ChannelCanvas>? pendingCanvas,
}) => ProviderScope(
  overrides: [
    currentPubkeyProvider.overrideWith((ref) => _currentPubkey),
    channelMembersProvider(channel.id).overrideWith((ref) => loadMembers()),
    channelSectionsProvider.overrideWith(
      () => _FakeChannelSectionsNotifier(const ChannelSectionStore()),
    ),
    agentOwnersProvider.overrideWithValue(
      const AsyncValue.data(<String, String>{}),
    ),
    channelCanvasProvider(channel.id).overrideWith(
      (ref) =>
          pendingCanvas ??
          ChannelCanvas(
            content: canvasContent,
            updatedAt: null,
            authorPubkey: null,
          ),
    ),
    channelActionsProvider.overrideWith(createChannelActions),
  ],
  child: MaterialApp(
    theme: AppTheme.light(),
    home: Builder(
      builder: (context) => Scaffold(
        body: TextButton(
          onPressed: () => showChannelActionsSheet(
            context: context,
            channel: channel,
            isUnread: false,
          ),
          child: const Text('Open actions'),
        ),
      ),
    ),
  ),
);

void main() {
  testWidgets('owner sees the complete regular-channel action set', (
    tester,
  ) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    for (final label in [
      'Star',
      'Mark Unread',
      'Move to section…',
      'Mute channel',
      'Manage channel',
      'Copy channel name',
      'Copy channel ID',
      'Leave channel',
      'Archive channel',
      'Delete channel',
    ]) {
      expect(find.text(label), findsOneWidget, reason: label);
    }

    final moveTop = tester.getTopLeft(find.text('Move to section…')).dy;
    final muteTop = tester.getTopLeft(find.text('Mute channel')).dy;
    final manageTop = tester.getTopLeft(find.text('Manage channel')).dy;
    final copyNameTop = tester.getTopLeft(find.text('Copy channel name')).dy;
    final copyIdTop = tester.getTopLeft(find.text('Copy channel ID')).dy;
    expect(moveTop, lessThan(muteTop));
    expect(muteTop, lessThan(manageTop));
    expect(manageTop, lessThan(copyNameTop));
    expect(copyNameTop, lessThan(copyIdTop));
  });

  testWidgets('unread channel uses the Mark Read label', (tester) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        isUnread: true,
        loadMembers: () async => const [],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Mark Read'), findsOneWidget);
    expect(find.text('Mark Unread'), findsNothing);
  });

  testWidgets('admin can archive but cannot delete', (tester) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'admin',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsOneWidget);
    expect(find.text('Delete channel'), findsNothing);
  });

  testWidgets('verified owner agent grants archive and delete', (tester) async {
    const agentPubkey = 'agent';
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        agentOwners: const AsyncValue.data({agentPubkey: _currentPubkey}),
        loadMembers: () async => [
          ChannelMember(
            pubkey: agentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsOneWidget);
    expect(find.text('Delete channel'), findsOneWidget);
  });

  testWidgets('unresolved identity grants no lifecycle actions', (
    tester,
  ) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        currentPubkey: null,
        loadMembers: () async => [
          ChannelMember(
            pubkey: 'ordinary-owner',
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Delete channel'), findsNothing);
  });

  testWidgets('archived owner can unarchive but cannot delete', (tester) async {
    late _FakeChannelActions actions;
    await tester.pumpWidget(
      _app(
        channel: _channel(isArchived: true),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
        createChannelActions: (ref) => actions = _FakeChannelActions(ref),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Unarchive channel'), findsOneWidget);
    expect(find.text('Delete channel'), findsNothing);

    await tester.tap(find.text('Unarchive channel'));
    await tester.pumpAndSettle();
    expect(find.text('Unarchive #general?'), findsOneWidget);
    await tester.tap(find.widgetWithText(FilledButton, 'Unarchive'));
    await tester.pumpAndSettle();

    expect(actions.unarchivedChannelId, 'channel-id');
  });

  testWidgets('owned non-owner agent grants no lifecycle actions', (
    tester,
  ) async {
    const agentPubkey = 'agent';
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        agentOwners: const AsyncValue.data({agentPubkey: _currentPubkey}),
        loadMembers: () async => [
          ChannelMember(
            pubkey: agentPubkey,
            role: 'bot',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Delete channel'), findsNothing);
  });

  testWidgets('agent ownership loading keeps lifecycle actions pending', (
    tester,
  ) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        agentOwners: const AsyncValue.loading(),
        loadMembers: () async => const [],
      ),
    );
    await tester.pump();

    expect(find.text('Loading channel actions…'), findsOneWidget);
    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Delete channel'), findsNothing);
  });

  testWidgets('member sees neither owner action', (tester) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Archive channel'), findsNothing);
    expect(find.text('Delete channel'), findsNothing);
    expect(find.text('Move to section…'), findsOneWidget);
    expect(find.text('Leave channel'), findsOneWidget);
  });

  testWidgets('shows loading and unavailable capability states', (
    tester,
  ) async {
    final pending = Completer<List<ChannelMember>>();
    await tester.pumpWidget(
      _app(channel: _channel(), loadMembers: () => pending.future),
    );
    await tester.pump();
    expect(find.text('Loading channel actions…'), findsOneWidget);

    pending.completeError(Exception('relay unavailable'));
    await tester.pumpAndSettle();
    expect(find.text('Channel actions unavailable'), findsOneWidget);
  });

  testWidgets('Manage contains editing and canvas, not mute or leave', (
    tester,
  ) async {
    await tester.pumpWidget(
      _modalApp(
        channel: _channel(),
        loadMembers: () async => const [],
        createChannelActions: (ref) => _FakeChannelActions(ref),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open actions'));
    await tester.pumpAndSettle();

    await tester.tap(find.text('Manage channel'));
    await tester.pumpAndSettle();

    final manageSheet = find.byType(ManageChannelSheet);
    expect(manageSheet, findsOneWidget);
    expect(
      find.descendant(
        of: manageSheet,
        matching: find.byKey(const ValueKey('manage-channel-name')),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(of: manageSheet, matching: find.text('Add Canvas')),
      findsOneWidget,
    );
    expect(
      find.descendant(of: manageSheet, matching: find.text('Mute channel')),
      findsNothing,
    );
    expect(
      find.descendant(of: manageSheet, matching: find.text('Leave channel')),
      findsNothing,
    );
    expect(
      find.descendant(of: manageSheet, matching: find.text('Topic')),
      findsNothing,
    );
    expect(
      find.descendant(of: manageSheet, matching: find.text('Purpose')),
      findsNothing,
    );
  });

  testWidgets('Manage refreshes metadata after saving from actions', (
    tester,
  ) async {
    await tester.pumpWidget(
      _modalApp(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
        createChannelActions: (ref) => _FakeChannelActions(
          ref,
          onUpdateChannel: (channelId, name, description) async {},
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open actions'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Manage channel'));
    await tester.pumpAndSettle();

    expect(
      tester.getSize(find.byType(BottomSheet).last).height,
      greaterThan(480),
    );
    await tester.enterText(
      find.byKey(const ValueKey('manage-channel-name')),
      'renamed',
    );
    await tester.pump();
    await tester.tap(find.byKey(const ValueKey('manage-channel-save-details')));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Manage channel'));
    await tester.pumpAndSettle();
    final nameField = tester.widget<TextField>(
      find.byKey(const ValueKey('manage-channel-name')),
    );
    expect(nameField.controller?.text, 'renamed');
  });

  testWidgets(
    'native channel editor shares the profile form and retains failed drafts',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      const bridge = MethodChannel('buzz/profile_text_editor');
      final presentations = <Map<Object?, Object?>>[];
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(bridge, (call) async {
            presentations.add(
              Map<Object?, Object?>.from(call.arguments as Map),
            );
            expect(call.method, 'presentChannel');
            return {
              'action': 'save',
              'name': 'renamed',
              'description': 'Updated description',
            };
          });
      addTearDown(
        () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(bridge, null),
      );
      var attempts = 0;
      await tester.pumpWidget(
        _modalApp(
          channel: _channel(),
          loadMembers: () async => [
            ChannelMember(
              pubkey: _currentPubkey,
              role: 'owner',
              joinedAt: DateTime(2025),
            ),
          ],
          createChannelActions: (ref) => _FakeChannelActions(
            ref,
            onUpdateChannel: (_, name, description) async {
              attempts++;
              if (attempts == 1) throw Exception('offline');
              expect(name, 'renamed');
              expect(description, 'Updated description');
            },
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Open actions'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Manage channel'));
      await tester.pumpAndSettle();
      expect(presentations, hasLength(2));
      expect(presentations.first['title'], 'Edit channel');
      expect(presentations.first['multiline'], false);
      expect(presentations.last['initialValue'], 'renamed');
      expect(presentations.last['allowUnchangedSubmission'], true);
      expect(presentations.last['description'], 'Updated description');
      expect(find.byType(ManageChannelSheet), findsNothing);
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets('native metadata editor opens before a cold canvas finishes', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    final canvas = Completer<ChannelCanvas>();
    const bridge = MethodChannel('buzz/profile_text_editor');
    var opened = false;
    var saved = false;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(bridge, (call) async {
          expect(canvas.isCompleted, isFalse);
          expect((call.arguments as Map)['canvasLoaded'], isFalse);
          opened = true;
          return {
            'action': 'save',
            'name': 'renamed',
            'description': 'New description',
          };
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(bridge, null),
    );
    await tester.pumpWidget(
      _modalApp(
        channel: _channel(),
        pendingCanvas: canvas.future,
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
        createChannelActions: (ref) => _FakeChannelActions(
          ref,
          onUpdateChannel: (_, name, description) async {
            saved = true;
          },
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open actions'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Manage channel'));
    await tester.pump();
    expect(opened, isTrue);
    expect(saved, isTrue);
    canvas.complete(
      const ChannelCanvas(content: null, updatedAt: null, authorPubkey: null),
    );
    await tester.pumpAndSettle();
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets('native canvas round trip retains both channel drafts', (
    tester,
  ) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    const bridge = MethodChannel('buzz/profile_text_editor');
    var channelPresentations = 0;
    var savedDetails = false;
    var savedCanvas = false;
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(bridge, (call) async {
          final args = Map<Object?, Object?>.from(call.arguments as Map);
          if (call.method == 'present') {
            expect(args['title'], 'Canvas');
            expect(args['multiline'], true);
            return 'New canvas';
          }
          expect(call.method, 'presentChannel');
          channelPresentations++;
          if (channelPresentations == 1) {
            return {
              'action': 'canvas',
              'name': 'Draft name',
              'description': 'Draft description',
            };
          }
          expect(args['canvasContent'], 'New canvas');
          expect(args['canvasLoaded'], true);
          expect(args['initialValue'], 'Draft name');
          expect(args['description'], 'Draft description');
          expect(args['originalName'], 'general');
          expect(args['originalDescription'], '');
          return {
            'action': 'save',
            'name': 'Draft name',
            'description': 'Draft description',
          };
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(bridge, null),
    );
    await tester.pumpWidget(
      _modalApp(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'owner',
            joinedAt: DateTime(2025),
          ),
        ],
        createChannelActions: (ref) => _FakeChannelActions(
          ref,
          onUpdateChannel: (_, name, description) async {
            expect(name, 'Draft name');
            expect(description, 'Draft description');
            savedDetails = true;
          },
          onSetCanvas: (content) async {
            expect(content, 'New canvas');
            savedCanvas = true;
          },
        ),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open actions'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Manage channel'));
    await tester.pumpAndSettle();
    expect(channelPresentations, 2);
    expect(savedDetails, isTrue);
    expect(savedCanvas, isTrue);
    expect(find.byType(ManageChannelSheet), findsNothing);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets(
    'canvas preview opens its content and removal retries without losing drafts',
    (tester) async {
      var removals = 0;
      await tester.pumpWidget(
        _modalApp(
          channel: _channel(),
          canvasContent: 'Shared team notes',
          loadMembers: () async => [
            ChannelMember(
              pubkey: _currentPubkey,
              role: 'owner',
              joinedAt: DateTime(2025),
            ),
          ],
          createChannelActions: (ref) => _FakeChannelActions(
            ref,
            onSetCanvas: (content) async {
              expect(content, '');
              if (++removals == 1) throw Exception('offline');
            },
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Open actions'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Manage channel'));
      await tester.pumpAndSettle();
      expect(find.text('Shared team notes'), findsOneWidget);
      expect(find.text('Add Canvas'), findsNothing);
      expect(find.text('Edit canvas'), findsNothing);
      await tester.enterText(
        find.byKey(const ValueKey('manage-channel-name')),
        'Draft name',
      );
      await tester.tap(find.byKey(const ValueKey('manage-channel-canvas')));
      await tester.pumpAndSettle();
      expect(
        tester
            .widget<TextField>(find.byKey(const ValueKey('text-editor-input')))
            .controller!
            .text,
        'Shared team notes',
      );
      Navigator.of(
        tester.element(find.byKey(const ValueKey('text-editor-input'))),
      ).pop();
      await tester.pumpAndSettle();
      for (var attempt = 0; attempt < 2; attempt++) {
        await tester.ensureVisible(find.text('Remove Canvas'));
        await tester.tap(find.text('Remove Canvas'));
        await tester.pumpAndSettle();
        expect(removals, attempt);
        await tester.tap(find.text('Remove'));
        await tester.pumpAndSettle();
        if (attempt == 0) {
          expect(find.text('Shared team notes'), findsOneWidget);
          expect(
            find.text("Couldn't remove the canvas. Try again."),
            findsOneWidget,
          );
        }
      }
      expect(find.text('Add Canvas'), findsOneWidget);
      expect(find.text('Remove Canvas'), findsNothing);
      expect(
        tester
            .widget<TextField>(
              find.byKey(const ValueKey('manage-channel-name')),
            )
            .controller!
            .text,
        'Draft name',
      );
    },
  );

  testWidgets(
    'native removal retains canvas on failure and clears after retry',
    (tester) async {
      debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
      addTearDown(() => debugDefaultTargetPlatformOverride = null);
      const bridge = MethodChannel('buzz/profile_text_editor');
      var presentations = 0;
      var attempts = 0;
      TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(bridge, (call) async {
            final args = Map<Object?, Object?>.from(call.arguments as Map);
            expect(call.method, 'presentChannel');
            expect(args['canvasLoaded'], true);
            expect(
              args['canvasContent'],
              presentations < 2 ? 'Existing canvas' : '',
            );
            if (presentations > 0) {
              expect(args['initialValue'], 'Draft name');
              expect(args['description'], 'Draft description');
            }
            if (++presentations == 3) return null;
            return {
              'action': 'removeCanvas',
              'name': 'Draft name',
              'description': 'Draft description',
            };
          });
      addTearDown(
        () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
            .setMockMethodCallHandler(bridge, null),
      );
      await tester.pumpWidget(
        _modalApp(
          channel: _channel(),
          canvasContent: 'Existing canvas',
          loadMembers: () async => [],
          createChannelActions: (ref) => _FakeChannelActions(
            ref,
            onSetCanvas: (content) async {
              expect(content, '');
              if (++attempts == 1) throw Exception('offline');
            },
          ),
        ),
      );
      await tester.pumpAndSettle();
      await tester.tap(find.text('Open actions'));
      await tester.pumpAndSettle();
      await tester.tap(find.text('Manage channel'));
      await tester.pumpAndSettle();
      expect(presentations, 3);
      expect(attempts, 2);
      debugDefaultTargetPlatformOverride = null;
    },
  );

  testWidgets('non-member cannot edit canvas from Manage channel', (
    tester,
  ) async {
    await tester.pumpWidget(
      _modalApp(
        channel: _channel(isMember: false),
        loadMembers: () async => const [],
        createChannelActions: (ref) => _FakeChannelActions(ref),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open actions'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Manage channel'));
    await tester.pumpAndSettle();

    final editCanvas = find.byKey(const ValueKey('manage-channel-canvas'));
    expect(editCanvas, findsOneWidget);
    expect(tester.widget<FilledButton>(editCanvas).onPressed, isNull);
  });

  testWidgets('regular members can move channels but cannot edit metadata', (
    tester,
  ) async {
    await tester.pumpWidget(
      _modalApp(
        channel: _channel(),
        loadMembers: () async => [
          ChannelMember(
            pubkey: _currentPubkey,
            role: 'member',
            joinedAt: DateTime(2025),
          ),
        ],
        createChannelActions: (ref) => _FakeChannelActions(ref),
      ),
    );
    await tester.pumpAndSettle();
    await tester.tap(find.text('Open actions'));
    await tester.pumpAndSettle();

    expect(find.text('Move to section…'), findsOneWidget);
    await tester.tap(find.text('Manage channel'));
    await tester.pumpAndSettle();

    final nameField = find.byKey(const ValueKey('manage-channel-name'));
    final descriptionField = find.byKey(
      const ValueKey('manage-channel-description'),
    );
    expect(tester.widget<TextField>(nameField).enabled, isFalse);
    expect(tester.widget<TextField>(descriptionField).enabled, isFalse);
    expect(
      tester
          .widget<FilledButton>(
            find.byKey(const ValueKey('manage-channel-canvas')),
          )
          .onPressed,
      isNotNull,
    );
  });

  testWidgets('DM omits quick actions, then shows mute and copy rows', (
    tester,
  ) async {
    await tester.pumpWidget(
      _app(
        channel: _channel(type: 'dm'),
        loadMembers: () async => const [],
      ),
    );
    await tester.pumpAndSettle();

    for (final label in [
      'Mute channel',
      'Copy channel name',
      'Copy channel ID',
    ]) {
      expect(find.text(label), findsOneWidget, reason: label);
    }
    for (final label in [
      'Star',
      'Unstar',
      'Mark Unread',
      'Mark Read',
      'Move to section…',
      'Manage channel',
      'Leave channel',
      'Archive channel',
      'Delete channel',
    ]) {
      expect(find.text(label), findsNothing, reason: label);
    }

    final muteTop = tester.getTopLeft(find.text('Mute channel')).dy;
    final copyNameTop = tester.getTopLeft(find.text('Copy channel name')).dy;
    final copyIdTop = tester.getTopLeft(find.text('Copy channel ID')).dy;
    expect(muteTop, lessThan(copyNameTop));
    expect(copyNameTop, lessThan(copyIdTop));
  });
}

class _FakeChannelSectionsNotifier extends ChannelSectionsNotifier {
  _FakeChannelSectionsNotifier(this._store);

  final ChannelSectionStore _store;

  @override
  ChannelSectionsState build() =>
      ChannelSectionsState(isReady: true, store: _store, version: 1);
}

class _FakeChannelActions extends ChannelActions {
  final Future<void> Function(
    String channelId,
    String? name,
    String? description,
  )?
  onUpdateChannel;

  final Future<void> Function(String)? onSetCanvas;

  _FakeChannelActions(Ref ref, {this.onUpdateChannel, this.onSetCanvas})
    : super(
        ref: ref,
        session: ref.read(relaySessionProvider.notifier),
        signedEventRelay: SignedEventRelay(
          session: ref.read(relaySessionProvider.notifier),
          nsec: null,
        ),
        currentPubkey: _currentPubkey,
      );

  @override
  Future<void> updateChannel({
    required String channelId,
    String? name,
    String? description,
  }) async {
    await onUpdateChannel?.call(channelId, name, description);
  }

  @override
  Future<void> setCanvas({
    required String channelId,
    required String content,
  }) async {
    await onSetCanvas?.call(content);
  }

  String? unarchivedChannelId;

  @override
  Future<void> leaveChannel(String channelId) async {}

  @override
  Future<void> unarchiveChannel(String channelId) async {
    unarchivedChannelId = channelId;
  }
}
