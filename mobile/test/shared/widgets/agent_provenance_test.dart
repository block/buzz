import 'dart:io';
import 'dart:ui' as ui;

import 'package:buzz/features/channels/message_content.dart';
import 'package:buzz/features/profile/user_profile_sheet.dart';
import 'package:buzz/shared/mentions/agent_identity_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/widgets/agent_provenance.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';

import '../../helpers/widget_helpers.dart';

const _agent =
    'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
const _owner =
    'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';
const _peer =
    'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';

void main() {
  setUp(() {
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(
          const MethodChannel('dev.fluttercommunity.plus/connectivity_status'),
          (_) async => null,
        );
  });
  testWidgets('shared avatar and mention use one accessible marker each', (
    tester,
  ) async {
    final semantics = tester.ensureSemantics();
    final cache = _Cache();
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          myPubkeyProvider.overrideWithValue(_owner),
          userCacheProvider.overrideWith(() => cache),
          agentOwnersProvider.overrideWith((ref) async => {_agent: _owner}),
        ],
        child: const Column(
          children: [
            AvatarImage(
              imageUrl: null,
              radius: 24,
              fallback: Text('A'),
              pubkey: _agent,
              isAgent: true,
            ),
            MessageContent(
              content: 'Hello @Agent',
              mentionNames: {_agent: 'Agent'},
              agentMentionPubkeys: {_agent},
            ),
          ],
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.byIcon(LucideIcons.cloud), findsNWidgets(2));
    expect(
      find.bySemanticsLabel(RegExp('Not managed on this device')),
      findsNWidgets(2),
    );
    expect(find.text('Agent'), findsOneWidget);
    cache.put(const UserProfile(pubkey: _agent, displayName: 'Agent'));
    await tester.pump();
    expect(
      find.byIcon(LucideIcons.cloud),
      findsNothing,
      reason: 'profile revocation beats stale directory ownership',
    );
    semantics.dispose();
  });

  testWidgets('peer, unknown owner, self and unknown viewer are unmarked', (
    tester,
  ) async {
    for (final viewer in [_peer, null, _agent]) {
      await tester.pumpWidget(
        WidgetHelpers.testable(
          overrides: [
            myPubkeyProvider.overrideWithValue(viewer),
            userCacheProvider.overrideWith(_Cache.new),
            agentOwnersProvider.overrideWith((ref) async => {_agent: _owner}),
          ],
          child: const AgentProvenance(pubkey: _agent),
        ),
      );
      await tester.pumpAndSettle();
      expect(find.byIcon(LucideIcons.cloud), findsNothing);
      await tester.pumpWidget(const SizedBox());
    }
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          myPubkeyProvider.overrideWithValue(_owner),
          userCacheProvider.overrideWith(_Cache.new),
          agentOwnersProvider.overrideWith(
            (ref) async => throw StateError('offline'),
          ),
        ],
        child: const AgentProvenance(pubkey: _peer),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.byIcon(LucideIcons.cloud), findsNothing);
  });

  testWidgets('profile hero preserves its full hit area and marker', (
    tester,
  ) async {
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          myPubkeyProvider.overrideWithValue(_owner),
          userCacheProvider.overrideWith(_Cache.new),
          agentOwnersProvider.overrideWith((ref) async => {_agent: _owner}),
        ],
        child: const UserProfileSheet(pubkey: _agent),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.byIcon(LucideIcons.cloud), findsOneWidget);
    final avatar = tester.getSize(
      find.byKey(const ValueKey('selected-profile-avatar')),
    );
    expect(avatar.width, greaterThan(100));
    expect(avatar.height, avatar.width);
  });

  testWidgets('phone-sized provenance review artifact', (tester) async {
    final output = Platform.environment['BUZZ_PROVENANCE_CAPTURE'];
    if (output != null) {
      await tester.runAsync(() async {
        await (FontLoader(
          'Inter',
        )..addFont(rootBundle.load('assets/fonts/InterVariable.ttf'))).load();
        await (FontLoader('packages/lucide_icons_flutter/Lucide')..addFont(
              rootBundle.load(
                'packages/lucide_icons_flutter/assets/lucide.ttf',
              ),
            ))
            .load();
      });
    }
    final boundary = GlobalKey();
    await tester.binding.setSurfaceSize(const Size(390, 440));
    addTearDown(() => tester.binding.setSurfaceSize(null));
    await tester.pumpWidget(
      WidgetHelpers.testable(
        overrides: [
          myPubkeyProvider.overrideWithValue(_owner),
          userCacheProvider.overrideWith(_Cache.new),
          agentOwnersProvider.overrideWith((ref) async => {_agent: _owner}),
        ],
        child: RepaintBoundary(
          key: boundary,
          child: const Material(
            child: Column(
              children: [
                ListTile(
                  title: Text('Your agent — not managed here'),
                  leading: AvatarImage(
                    imageUrl: null,
                    radius: 24,
                    fallback: Text('A'),
                    pubkey: _agent,
                    isAgent: true,
                  ),
                ),
                MessageContent(
                  content: 'Hello @Agent',
                  mentionNames: {_agent: 'Agent'},
                  agentMentionPubkeys: {_agent},
                ),
                ListTile(
                  title: Text('Teammate’s agent — no provenance claim'),
                  leading: AvatarImage(
                    imageUrl: null,
                    radius: 24,
                    fallback: Text('P'),
                    pubkey: _peer,
                    isAgent: true,
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    expect(find.byIcon(LucideIcons.cloud), findsNWidgets(2));
    if (output != null) {
      await tester.runAsync(() async {
        final image =
            await (boundary.currentContext!.findRenderObject()
                    as RenderRepaintBoundary)
                .toImage(pixelRatio: 2);
        final bytes = await image.toByteData(format: ui.ImageByteFormat.png);
        await File(output).writeAsBytes(bytes!.buffer.asUint8List());
        image.dispose();
      });
    }
  });
}

class _Cache extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => const {
    _agent: UserProfile(
      pubkey: _agent,
      displayName: 'Agent',
      ownerPubkey: _owner,
    ),
    _peer: UserProfile(pubkey: _peer, displayName: 'Peer', ownerPubkey: _peer),
  };
  @override
  Future<bool> preload(List<String> pubkeys) async => true;
}
