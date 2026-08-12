import 'dart:async';

import 'package:buzz/features/invites/invite_create_page.dart';
import 'package:buzz/features/invites/invite_create_provider.dart';
import 'package:buzz/shared/community/community_membership_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:flutter/material.dart';
import 'package:flutter/services.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:nostr/nostr.dart' as nostr;

void main() {
  const owner =
      'aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa';
  const alice =
      'bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb';

  testWidgets('owner can invite a pasted npub and share generated link', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(430, 1100);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final actions = _FakeInviteActions();
    String? sharedUrl;
    final hapticCalls = <MethodCall>[];
    TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
        .setMockMethodCallHandler(SystemChannels.platform, (call) async {
          if (call.method == 'HapticFeedback.vibrate') hapticCalls.add(call);
          return null;
        });
    addTearDown(
      () => TestDefaultBinaryMessengerBinding.instance.defaultBinaryMessenger
          .setMockMethodCallHandler(SystemChannels.platform, null),
    );

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          currentCommunityRoleProvider.overrideWithValue(
            const AsyncData<CommunityMemberRole?>(CommunityMemberRole.owner),
          ),
          communityMembershipProvider.overrideWith(
            (ref) async => const CommunityMembershipSnapshot(
              snapshotFound: true,
              members: [
                CommunityMember(pubkey: owner, role: CommunityMemberRole.owner),
              ],
            ),
          ),
          myPubkeyProvider.overrideWithValue(owner),
          communityInviteDirectoryProvider.overrideWith(
            (ref) async => const [
              CommunityInviteDirectoryUser(
                pubkey: alice,
                displayName: 'Alice',
                nip05Handle: 'alice@example.com',
              ),
            ],
          ),
          communityInviteDirectorySearchProvider.overrideWith(
            (ref, query) async => const [],
          ),
          communityInviteProfileProvider.overrideWith(
            (ref, pubkey) async => CommunityInviteDirectoryUser(
              pubkey: pubkey,
              displayName: 'Resolved person',
              nip05Handle: 'person@example.com',
            ),
          ),
          communityInviteActionsProvider.overrideWithValue(actions),
          shareCommunityInviteProvider.overrideWithValue((url, origin) async {
            sharedUrl = url;
          }),
        ],
        child: MaterialApp(
          theme: AppTheme.light().copyWith(platform: TargetPlatform.iOS),
          home: const CommunityInvitePage(),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(actions.mintRequests, [(defaultCommunityInviteTtlSeconds, null)]);
    expect(
      find.text('Add someone directly or share a link they can use to join.'),
      findsNothing,
    );
    expect(find.text('Invite people'), findsNothing);
    expect(find.text('Search by name or paste an npub.'), findsNothing);
    expect(find.text('Alice'), findsNothing);
    expect(
      find.byKey(const Key('community-invite-person-$alice')),
      findsNothing,
    );
    expect(find.text('https://relay.example.com/invite/1'), findsNothing);
    expect(find.text('Invite link ready'), findsNothing);
    expect(find.text('Invite link'), findsNothing);
    expect(find.text('Creating invite link…'), findsNothing);
    expect(find.text('Or use npub'), findsOneWidget);
    expect(find.text('Search npub'), findsOneWidget);
    expect(find.text('Copy'), findsOneWidget);
    expect(find.text('Share'), findsOneWidget);
    expect(find.text('Messages'), findsNothing);
    expect(find.text('Mail'), findsNothing);
    expect(find.text('WhatsApp'), findsNothing);
    expect(find.text('Gmail'), findsNothing);
    expect(find.text('Telegram'), findsNothing);
    expect(find.text('X'), findsNothing);
    expect(
      tester.getSize(find.byKey(const Key('community-invite-search'))).width,
      greaterThan(350),
    );
    final shareSize = tester.getSize(
      find.byKey(const Key('community-invite-share-link')),
    );
    final copySize = tester.getSize(
      find.byKey(const Key('community-invite-copy-link')),
    );
    expect(shareSize, copySize);
    expect(shareSize.height, 68 + (Grid.xxs * 2));
    expect(
      tester
          .getTopLeft(find.byKey(const Key('community-invite-share-link')))
          .dx,
      lessThan(
        tester
            .getTopLeft(find.byKey(const Key('community-invite-copy-link')))
            .dx,
      ),
    );
    expect(tester.widget<Icon>(find.byIcon(LucideIcons.share2)).size, 22);
    expect(tester.widget<Icon>(find.byIcon(LucideIcons.copy)).size, 22);
    final settingsDivider = find.descendant(
      of: find.byKey(const Key('community-invite-link-settings')),
      matching: find.byType(Divider),
    );
    final divider = tester.widget<Divider>(settingsDivider);
    expect(divider.indent, Grid.xs);
    expect(divider.endIndent, Grid.xs);
    expect(
      tester.getRect(settingsDivider).left + divider.indent!,
      closeTo(tester.getRect(find.text('Expires after')).left, 0.5),
    );

    const pastedPubkey =
        'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';
    final npub = nostr.Nip19.encode(
      prefix: nostr.Nip19Prefix.npub,
      data: pastedPubkey,
    );
    await tester.enterText(
      find.byKey(const Key('community-invite-search')),
      npub,
    );
    await tester.pumpAndSettle();
    expect(
      find.byKey(const Key('community-invite-resolved-$pastedPubkey')),
      findsOneWidget,
    );
    expect(find.text(shortCommunityInviteNpub(pastedPubkey)), findsOneWidget);
    expect(find.text('Resolved person'), findsNothing);
    expect(find.text('person@example.com'), findsNothing);
    expect(
      find.byKey(const Key('community-invite-person-$pastedPubkey')),
      findsNothing,
    );
    expect(find.text('Role'), findsOneWidget);
    expect(find.byIcon(LucideIcons.shield), findsNothing);
    final personDivider = find.descendant(
      of: find.byKey(const Key('community-invite-person-card')),
      matching: find.byType(Divider),
    );
    expect(personDivider, findsOneWidget);
    final personDividerWidget = tester.widget<Divider>(personDivider);
    expect(personDividerWidget.indent, Grid.xs);
    expect(personDividerWidget.endIndent, Grid.xs);
    await tester.tap(find.byKey(const Key('community-invite-submit')));
    await tester.pumpAndSettle();

    expect(actions.memberInvites, hasLength(1));
    expect(actions.memberInvites.single.$1, [pastedPubkey]);
    expect(actions.memberInvites.single.$2, CommunityMemberRole.member);

    await tester.ensureVisible(
      find.byKey(const Key('community-invite-share-link')),
    );
    await tester.tap(find.byKey(const Key('community-invite-share-link')));
    await tester.pumpAndSettle();
    expect(sharedUrl, 'https://relay.example.com/invite/1');

    await tester.ensureVisible(
      find.byKey(const Key('community-invite-copy-link')),
    );
    await tester.tap(find.byKey(const Key('community-invite-copy-link')));
    await tester.pumpAndSettle();
    expect(
      hapticCalls.map((call) => call.arguments),
      everyElement('HapticFeedbackType.lightImpact'),
    );
    expect(hapticCalls, hasLength(2));
  });

  testWidgets('pasted npub remains inviteable without profile metadata', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(430, 1100);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final actions = _FakeInviteActions();
    final profileResolution = Completer<CommunityInviteDirectoryUser?>();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          currentCommunityRoleProvider.overrideWithValue(
            const AsyncData<CommunityMemberRole?>(CommunityMemberRole.admin),
          ),
          communityMembershipProvider.overrideWith(
            (ref) async => const CommunityMembershipSnapshot(
              snapshotFound: true,
              members: [],
            ),
          ),
          myPubkeyProvider.overrideWithValue(owner),
          communityInviteProfileProvider.overrideWith(
            (ref, pubkey) => profileResolution.future,
          ),
          communityInviteActionsProvider.overrideWithValue(actions),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: const CommunityInvitePage(),
        ),
      ),
    );
    await tester.pumpAndSettle();

    const pastedPubkey =
        'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';
    final npub = nostr.Nip19.encode(
      prefix: nostr.Nip19Prefix.npub,
      data: pastedPubkey,
    );
    await tester.enterText(
      find.byKey(const Key('community-invite-search')),
      npub,
    );
    await tester.pump();

    expect(
      find.byKey(const Key('community-invite-resolving-$pastedPubkey')),
      findsOneWidget,
    );
    expect(find.byKey(const Key('community-invite-submit')), findsNothing);
    expect(actions.memberInvites, isEmpty);

    profileResolution.complete(
      const CommunityInviteDirectoryUser(pubkey: pastedPubkey),
    );
    await tester.pumpAndSettle();

    expect(
      find.byKey(const Key('community-invite-resolved-$pastedPubkey')),
      findsOneWidget,
    );
    expect(find.text('Profile not found'), findsNothing);
    expect(find.text(shortCommunityInviteNpub(pastedPubkey)), findsOneWidget);
    expect(find.byKey(const Key('community-invite-submit')), findsOneWidget);

    await tester.tap(find.byKey(const Key('community-invite-submit')));
    await tester.pumpAndSettle();

    expect(actions.memberInvites, hasLength(1));
    expect(actions.memberInvites.single.$1, [pastedPubkey]);
    expect(actions.memberInvites.single.$2, CommunityMemberRole.member);
  });

  testWidgets('invite completion does not update state after page disposal', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(430, 1100);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final actions = _FakeInviteActions()
      ..memberInviteCompleter = Completer<void>();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          currentCommunityRoleProvider.overrideWithValue(
            const AsyncData<CommunityMemberRole?>(CommunityMemberRole.admin),
          ),
          communityMembershipProvider.overrideWith(
            (ref) async => const CommunityMembershipSnapshot(
              snapshotFound: true,
              members: [],
            ),
          ),
          myPubkeyProvider.overrideWithValue(owner),
          communityInviteProfileProvider.overrideWith(
            (ref, pubkey) async => CommunityInviteDirectoryUser(pubkey: pubkey),
          ),
          communityInviteActionsProvider.overrideWithValue(actions),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: const CommunityInvitePage(),
        ),
      ),
    );
    await tester.pumpAndSettle();

    const pastedPubkey =
        'cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc';
    final npub = nostr.Nip19.encode(
      prefix: nostr.Nip19Prefix.npub,
      data: pastedPubkey,
    );
    await tester.enterText(
      find.byKey(const Key('community-invite-search')),
      npub,
    );
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('community-invite-submit')));
    await tester.pump();

    await tester.pumpWidget(const SizedBox.shrink());
    actions.memberInviteCompleter!.complete();
    await tester.pump();

    expect(tester.takeException(), isNull);
  });

  testWidgets('link settings remint with desktop expiry and use options', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(430, 1100);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);
    final actions = _FakeInviteActions();

    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          currentCommunityRoleProvider.overrideWithValue(
            const AsyncData<CommunityMemberRole?>(CommunityMemberRole.admin),
          ),
          communityMembershipProvider.overrideWith(
            (ref) async => const CommunityMembershipSnapshot(
              snapshotFound: true,
              members: [],
            ),
          ),
          communityInviteDirectoryProvider.overrideWith(
            (ref) async => const [],
          ),
          communityInviteActionsProvider.overrideWithValue(actions),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: const CommunityInvitePage(),
        ),
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(
      find.byKey(const Key('community-invite-max-uses-setting')),
    );
    await tester.pumpAndSettle();
    expect(find.text('Limit number of uses'), findsNWidgets(2));
    expect(find.byIcon(LucideIcons.check), findsOneWidget);
    await tester.tap(find.byKey(const Key('community-invite-option-5-uses')));
    await tester.pumpAndSettle();

    expect(actions.mintRequests, [
      (defaultCommunityInviteTtlSeconds, null),
      (defaultCommunityInviteTtlSeconds, 5),
    ]);

    await tester.tap(find.byKey(const Key('community-invite-expiry-setting')));
    await tester.pumpAndSettle();
    expect(find.text('Expires after'), findsNWidgets(2));
    expect(find.byIcon(LucideIcons.check), findsOneWidget);
    await tester.tap(find.byKey(const Key('community-invite-option-7-days')));
    await tester.pumpAndSettle();

    expect(actions.mintRequests.last, (7 * 24 * 60 * 60, 5));
  });

  testWidgets('plain members cannot open invite tools', (tester) async {
    await tester.pumpWidget(
      ProviderScope(
        overrides: [
          currentCommunityRoleProvider.overrideWithValue(
            const AsyncData<CommunityMemberRole?>(CommunityMemberRole.member),
          ),
        ],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: const CommunityInvitePage(),
        ),
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('Invite access required'), findsOneWidget);
    expect(find.byKey(const Key('community-invite-search')), findsNothing);
  });
}

class _FakeInviteActions implements CommunityInviteActions {
  final List<(int, int?)> mintRequests = [];
  final List<(List<String>, CommunityMemberRole)> memberInvites = [];
  Completer<void>? memberInviteCompleter;

  @override
  Future<void> inviteMembers({
    required Iterable<String> pubkeys,
    required CommunityMemberRole role,
  }) async {
    memberInvites.add((pubkeys.toList(), role));
    await memberInviteCompleter?.future;
  }

  @override
  Future<MintedCommunityInvite> mintInvite({
    required int ttlSeconds,
    required int? maxUses,
  }) async {
    mintRequests.add((ttlSeconds, maxUses));
    return MintedCommunityInvite(
      code: '${mintRequests.length}',
      expiresAt: 12345,
      url: 'https://relay.example.com/invite/${mintRequests.length}',
      maxUses: maxUses,
      usesRemaining: maxUses,
    );
  }
}
