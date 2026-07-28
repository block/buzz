import 'dart:async';
import 'dart:math';

import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:hooks_riverpod/misc.dart';
import 'package:lucide_icons_flutter/lucide_icons.dart';
import 'package:buzz/features/channels/channel.dart';
import 'package:buzz/features/channels/channel_management_provider.dart';
import 'package:buzz/features/channels/channels_page.dart';
import 'package:buzz/features/channels/channels_provider.dart';
import 'package:buzz/features/channels/read_state/read_state_provider.dart';
import 'package:buzz/features/channels/unread_badge/observed_unread_event.dart';
import 'package:buzz/features/profile/profile_avatar.dart';
import 'package:buzz/features/profile/profile_provider.dart';
import 'package:buzz/features/profile/user_profile.dart';
import 'package:buzz/shared/auth/auth.dart';
import 'package:buzz/shared/community/community_icon_provider.dart';
import 'package:buzz/shared/relay/relay.dart';
import 'package:buzz/shared/theme/theme.dart';
import 'package:buzz/shared/widgets/avatar_image.dart';
import 'package:buzz/shared/widgets/skeleton.dart';

void main() {
  Widget buildTestable({
    required List<Override> overrides,
    bool previewDirectory = false,
    double keyboardInset = 0,
    bool disableAnimations = false,
    Map<String, String?> communityIcons = const {},
    ValueChanged<String>? onCommunityIconLoad,
    TextScaler textScaler = TextScaler.noScaling,
  }) {
    return ProviderScope(
      overrides: [
        // Provide a fake profile and presence so the avatar doesn't hit the network.
        profileProvider.overrideWith(() => _FakeProfileNotifier()),
        presenceProvider.overrideWith(() => _FakePresenceNotifier()),
        communityIconProvider.overrideWith((ref, relayUrl) async {
          onCommunityIconLoad?.call(relayUrl);
          return communityIcons[relayUrl];
        }),
        dmDirectoryPreviewEnabledProvider.overrideWith(
          (ref) => previewDirectory,
        ),
        ...overrides,
      ],
      child: MaterialApp(
        theme: AppTheme.light(),
        builder: (context, child) => MediaQuery(
          data: MediaQuery.of(context).copyWith(
            disableAnimations: disableAnimations,
            textScaler: textScaler,
            viewInsets: EdgeInsets.only(bottom: keyboardInset),
          ),
          child: child!,
        ),
        home: const Stack(
          children: [
            ChannelsPage(settingsPageBuilder: _buildSettingsPage),
            Positioned.fill(
              child: ChannelQuickActionsLauncher(
                visible: true,
                navigationBarHeight: 60,
                navigationBarBottomGap: 12,
                navigationBarWidth: 218,
                systemBottomInset: 0,
                rightInset: 16,
              ),
            ),
          ],
        ),
      ),
    );
  }

  final testChannels = [
    Channel(
      id: '1',
      name: 'general',
      channelType: 'stream',
      visibility: 'open',
      description: 'General discussion',
      createdBy: 'abc',
      createdAt: DateTime(2025),
      memberCount: 10,
      isMember: true,
    ),
    Channel(
      id: '2',
      name: 'design-forum',
      channelType: 'forum',
      visibility: 'open',
      description: 'Discuss designs',
      createdBy: 'abc',
      createdAt: DateTime(2025),
      memberCount: 3,
      isMember: true,
    ),
    Channel(
      id: '3',
      name: 'DM',
      channelType: 'dm',
      visibility: 'open',
      description: 'Direct message',
      createdBy: 'abc',
      createdAt: DateTime(2025),
      memberCount: 2,
      participants: const ['Test', 'Alice'],
      participantPubkeys: const ['aabb', 'alice'],
      isMember: true,
    ),
  ];

  testWidgets('shows grouped channel list when data loads', (tester) async {
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(testChannels)),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('general'), findsOneWidget);
    expect(find.text('design-forum'), findsNothing);
    expect(find.text('Alice'), findsOneWidget);
    expect(find.text('Channels'), findsOneWidget);
    expect(find.text('FORUMS'), findsNothing);
    expect(find.text('DMs'), findsOneWidget);
    expect(find.text('Community'), findsOneWidget);
    expect(find.byTooltip('Create or start conversation'), findsOneWidget);

    for (final label in ['general', 'Alice']) {
      final text = tester.widget<Text>(find.text(label));
      expect(text.style?.fontSize, contentListTitleTextStyle.fontSize);
      expect(text.style?.height, contentListTitleTextStyle.height);
    }
    final sectionTitle = tester.widget<Text>(find.text('Channels'));
    expect(sectionTitle.style?.fontSize, contentListTitleTextStyle.fontSize);
    expect(sectionTitle.style?.fontWeight, FontWeight.w600);
  });

  testWidgets('aligns the top, section, row, and skeleton label columns', (
    tester,
  ) async {
    final relaySession = _ReconnectingRelaySession();
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(testChannels)),
          relaySessionProvider.overrideWith(() => relaySession),
        ],
      ),
    );
    relaySession.connect();
    await tester.pumpAndSettle();

    final topLabelX = tester.getTopLeft(find.text('Community')).dx;
    final sectionLabelX = tester.getTopLeft(find.text('Channels')).dx;
    final rowLabelX = tester.getTopLeft(find.text('general')).dx;
    expect(topLabelX, sectionLabelX);
    expect(sectionLabelX, rowLabelX);

    relaySession.setReconnecting();
    await tester.pump();
    await tester.pump(const Duration(seconds: 2));
    await tester.pump();

    final skeletonSectionLabelX = tester
        .getTopLeft(
          find.byKey(const Key('channels-skeleton-section-label')).first,
        )
        .dx;
    final skeletonRowLabelX = tester
        .getTopLeft(
          find.byKey(const Key('channels-skeleton-row-label-0')).first,
        )
        .dx;
    expect(skeletonSectionLabelX, skeletonRowLabelX);
    expect(skeletonSectionLabelX, sectionLabelX);
  });

  testWidgets('reveals channel content from same-slot reconnect skeletons', (
    tester,
  ) async {
    final relaySession = _ReconnectingRelaySession();
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(testChannels)),
          relaySessionProvider.overrideWith(() => relaySession),
        ],
      ),
    );
    await tester.pump();
    await tester.pump(const Duration(seconds: 2));
    await tester.pump();

    final skeleton = find.byKey(const Key('channels-connection-skeleton'));
    expect(skeleton, findsOneWidget);
    expect(
      find.descendant(of: skeleton, matching: find.byType(SkeletonBar)),
      findsWidgets,
    );
    expect(
      find.descendant(
        of: skeleton,
        matching: find.byType(CircularProgressIndicator),
      ),
      findsNothing,
    );
    expect(
      tester
          .widget<Opacity>(find.byKey(const Key('skeleton-reveal-placeholder')))
          .opacity,
      1,
    );
    expect(
      tester
          .widget<Opacity>(find.byKey(const Key('skeleton-reveal-content')))
          .opacity,
      0,
    );

    relaySession.connect();
    await tester.pump();
    await tester.pump();
    expect(
      tester.widget<SkeletonReveal>(find.byType(SkeletonReveal)).loading,
      isFalse,
    );
    await tester.pump(const Duration(milliseconds: 200));

    expect(
      tester
          .widget<Opacity>(find.byKey(const Key('skeleton-reveal-placeholder')))
          .opacity,
      closeTo(0.5, 0.01),
    );
    expect(
      tester
          .widget<Opacity>(find.byKey(const Key('skeleton-reveal-content')))
          .opacity,
      closeTo(0.5, 0.01),
    );

    await tester.pump(const Duration(milliseconds: 200));
    expect(find.text('general'), findsOneWidget);
  });

  testWidgets('announces neutral loading outside connection transitions', (
    tester,
  ) async {
    final relaySession = _ReconnectingRelaySession(
      initialStatus: SessionStatus.connected,
    );
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _LoadingNotifier()),
          relaySessionProvider.overrideWith(() => relaySession),
        ],
      ),
    );
    await tester.pump();

    expect(
      tester
          .widget<Semantics>(
            find.byKey(const Key('channels-connection-skeleton')),
          )
          .properties
          .label,
      'Loading',
    );
  });

  testWidgets('opens the settings page supplied by the app layer', (
    tester,
  ) async {
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(testChannels)),
        ],
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byType(ProfileAvatar));
    await tester.pumpAndSettle();

    expect(find.text('Injected settings'), findsOneWidget);
  });

  testWidgets('community switcher separates selection from edit removal', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 844);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);

    final communities = [
      Community(
        id: 'alpha',
        name: 'Alpha',
        relayUrl: 'wss://alpha.example.com',
        addedAt: DateTime(2025),
      ),
      Community(
        id: 'bravo',
        name: 'Bravo',
        relayUrl: 'wss://bravo.example.com',
        addedAt: DateTime(2025),
      ),
    ];
    final communityNotifier = _FakeCommunityListNotifier(communities);

    await tester.pumpWidget(
      buildTestable(
        communityIcons: const {
          'wss://alpha.example.com':
              'data:image/png;base64,'
              'iVBORw0KGgoAAAANSUhEUgAAAAEAAAABCAQAAAC1HAwCAAAAC0lEQVR42mNk+A8AAQUBAScY42YAAAAASUVORK5CYII=',
        },
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(testChannels)),
          communityListProvider.overrideWith(() => communityNotifier),
          activeCommunityProvider.overrideWith(
            (ref) async => communities.first,
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.text('Alpha'));
    await tester.pumpAndSettle();

    expect(find.text('Switch Community'), findsOneWidget);
    final options = find.byKey(const Key('community-switcher-options'));
    expect(options, findsOneWidget);
    final editButton = find.byKey(const Key('community-switcher-edit'));
    expect(
      tester.getRect(options).top - tester.getRect(editButton).bottom,
      closeTo(8, 0.01),
    );
    expect(tester.getSize(editButton).height, 32);
    expect(find.text('alpha.example.com'), findsOneWidget);
    expect(find.text('bravo.example.com'), findsOneWidget);
    expect(find.text('Rename'), findsNothing);
    expect(find.byIcon(LucideIcons.ellipsisVertical), findsNothing);
    expect(find.text('Edit'), findsOneWidget);
    expect(find.byIcon(LucideIcons.trash2), findsNothing);
    expect(
      tester.getSize(find.byKey(const Key('community-switcher-avatar-alpha'))),
      const Size.square(36),
    );
    final alphaAvatar = tester.widget<AvatarImage>(
      find.descendant(
        of: find.byKey(const Key('community-switcher-avatar-alpha')),
        matching: find.byType(AvatarImage),
      ),
    );
    expect(alphaAvatar.imageUrl, startsWith('data:image/png;base64,'));
    final activeSelection = find.byKey(
      const Key('community-switcher-selection-alpha'),
    );
    final inactiveSelection = find.byKey(
      const Key('community-switcher-selection-bravo'),
    );
    expect(tester.getSize(activeSelection), const Size.square(40));
    expect(
      tester.getSize(find.byKey(const Key('community-switcher-circle-alpha'))),
      const Size.square(24),
    );
    expect(
      find.descendant(
        of: activeSelection,
        matching: find.byIcon(LucideIcons.check),
      ),
      findsOneWidget,
    );
    expect(
      find.descendant(
        of: inactiveSelection,
        matching: find.byIcon(LucideIcons.check),
      ),
      findsNothing,
    );
    expect(
      find.descendant(of: options, matching: find.byType(Divider)),
      findsNWidgets(2),
    );

    await tester.tap(find.text('Edit'));
    await tester.pump();

    final activeAction = find.byKey(
      const Key('community-switcher-action-alpha'),
    );
    final actionSwitcher = tester.widget<AnimatedSwitcher>(
      find.descendant(
        of: activeAction,
        matching: find.byType(AnimatedSwitcher),
      ),
    );
    expect(actionSwitcher.duration, const Duration(milliseconds: 250));
    expect(activeSelection, findsOneWidget);
    expect(
      find.byKey(const Key('community-switcher-remove-alpha')),
      findsOneWidget,
    );

    await tester.pump(const Duration(milliseconds: 125));

    final transitioningOpacities = tester
        .widgetList<Opacity>(
          find.descendant(of: activeAction, matching: find.byType(Opacity)),
        )
        .map((opacity) => opacity.opacity);
    expect(
      transitioningOpacities.any((opacity) => opacity > 0 && opacity < 1),
      isTrue,
    );
    expect(
      find.descendant(of: activeAction, matching: find.byType(ImageFiltered)),
      findsWidgets,
    );

    await tester.pumpAndSettle();

    expect(find.text('Done'), findsOneWidget);
    expect(find.byIcon(LucideIcons.trash2), findsNWidgets(2));
    expect(activeSelection, findsNothing);
    expect(inactiveSelection, findsNothing);

    await tester.tap(find.byTooltip('Remove Bravo'));
    await tester.pumpAndSettle();

    expect(find.text('Remove community?'), findsOneWidget);
    expect(
      find.text(
        'Are you sure you want to remove “Bravo”? '
        'You can pair with it again later.',
      ),
      findsOneWidget,
    );

    await tester.tap(find.widgetWithText(FilledButton, 'Remove'));
    await tester.pumpAndSettle();

    expect(communityNotifier.removedIds, ['bravo']);
    expect(find.text('Switch Community'), findsOneWidget);
    expect(find.text('Bravo'), findsNothing);
    expect(find.text('Done'), findsOneWidget);

    await tester.tap(find.text('Done'));
    await tester.pumpAndSettle();

    expect(find.text('Edit'), findsOneWidget);
    expect(
      find.byKey(const Key('community-switcher-selection-alpha')),
      findsOneWidget,
    );
  });

  testWidgets('opening the community switcher refreshes visible icons', (
    tester,
  ) async {
    final community = Community(
      id: 'alpha',
      name: 'Alpha',
      relayUrl: 'wss://alpha.example.com',
      addedAt: DateTime(2025),
    );
    var iconLoads = 0;

    await tester.pumpWidget(
      buildTestable(
        onCommunityIconLoad: (_) => iconLoads++,
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(testChannels)),
          communityListProvider.overrideWith(
            () => _FakeCommunityListNotifier([community]),
          ),
          activeCommunityProvider.overrideWith((ref) async => community),
        ],
      ),
    );
    await tester.pumpAndSettle();
    final initialIconLoads = iconLoads;

    await tester.tap(find.text('Alpha'));
    await tester.pumpAndSettle();

    expect(iconLoads, greaterThan(initialIconLoads));
  });

  testWidgets('community switcher header grows with accessible text', (
    tester,
  ) async {
    final community = Community(
      id: 'alpha',
      name: 'Alpha',
      relayUrl: 'wss://alpha.example.com',
      addedAt: DateTime(2025),
    );

    await tester.pumpWidget(
      buildTestable(
        textScaler: TextScaler.linear(2),
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(testChannels)),
          communityListProvider.overrideWith(
            () => _FakeCommunityListNotifier([community]),
          ),
          activeCommunityProvider.overrideWith((ref) async => community),
        ],
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.text('Alpha'));
    await tester.pumpAndSettle();

    expect(tester.takeException(), isNull);
    expect(
      tester.getSize(find.byKey(const Key('community-switcher-title'))).height,
      greaterThan(32),
    );
  });

  testWidgets('community switcher disables icon motion when requested', (
    tester,
  ) async {
    final communities = [
      Community(
        id: 'alpha',
        name: 'Alpha',
        relayUrl: 'wss://alpha.example.com',
        addedAt: DateTime(2025),
      ),
    ];

    await tester.pumpWidget(
      buildTestable(
        disableAnimations: true,
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(testChannels)),
          communityListProvider.overrideWith(
            () => _FakeCommunityListNotifier(communities),
          ),
          activeCommunityProvider.overrideWith(
            (ref) async => communities.first,
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.text('Alpha'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Edit'));
    await tester.pump();

    final actionSwitcher = tester.widget<AnimatedSwitcher>(
      find.descendant(
        of: find.byKey(const Key('community-switcher-action-alpha')),
        matching: find.byType(AnimatedSwitcher),
      ),
    );
    expect(actionSwitcher.duration, Duration.zero);
    expect(
      find.byKey(const Key('community-switcher-selection-alpha')),
      findsNothing,
    );
    expect(
      find.byKey(const Key('community-switcher-remove-alpha')),
      findsOneWidget,
    );
  });

  testWidgets('quick actions slide behind navigation when leaving home', (
    tester,
  ) async {
    Widget buildLauncher({required bool visible}) {
      return ProviderScope(
        overrides: [profileProvider.overrideWith(() => _FakeProfileNotifier())],
        child: MaterialApp(
          theme: AppTheme.light(),
          home: Stack(
            children: [
              Positioned.fill(
                child: ChannelQuickActionsLauncher(
                  visible: visible,
                  navigationBarHeight: 60,
                  navigationBarBottomGap: 12,
                  navigationBarWidth: 218,
                  systemBottomInset: 0,
                  rightInset: 16,
                ),
              ),
            ],
          ),
        ),
      );
    }

    await tester.pumpWidget(buildLauncher(visible: true));
    await tester.pumpAndSettle();
    Transform motionTransform() => tester.widget<Transform>(
      find.byKey(const Key('channel-quick-actions-transform')),
    );

    expect(motionTransform().transform.getTranslation().x, 0);

    await tester.pumpWidget(buildLauncher(visible: false));
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 110));
    final midpoint = motionTransform().transform.getTranslation().x;
    expect(midpoint, lessThan(0));
    expect(midpoint, greaterThan(-279));

    await tester.pumpAndSettle();
    expect(motionTransform().transform.getTranslation().x, closeTo(-279, 0.01));
    final hiddenOpacity = tester.widget<Opacity>(
      find.byKey(const Key('channel-quick-actions-opacity')),
    );
    expect(hiddenOpacity.opacity, 0);
    final hiddenPointerGate = tester.widget<IgnorePointer>(
      find
          .ancestor(
            of: find.byKey(const Key('channel-quick-actions-transform')),
            matching: find.byType(IgnorePointer),
          )
          .first,
    );
    expect(hiddenPointerGate.ignoring, isTrue);
  });

  testWidgets('quick actions spring into spaced muted cards', (tester) async {
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(testChannels)),
        ],
      ),
    );
    await tester.pumpAndSettle();

    final surface = find.byKey(const Key('quick-actions-surface'));
    expect(tester.getSize(surface), const Size.square(56));

    await tester.tap(find.byTooltip('Create or start conversation'));
    await tester.pump();

    var largestHeight = tester.getSize(surface).height;
    for (var frame = 0; frame < 20; frame++) {
      await tester.pump(const Duration(milliseconds: 16));
      largestHeight = max(largestHeight, tester.getSize(surface).height);
    }
    await tester.pumpAndSettle();

    expect(largestHeight, greaterThan(160));
    expect(tester.getSize(surface).height, closeTo(160, 0.01));
    final screenWidth = MediaQuery.sizeOf(tester.element(surface)).width;
    final surfaceRect = tester.getRect(surface);
    expect(surfaceRect.left, closeTo(20, 0.01));
    expect(surfaceRect.right, closeTo(screenWidth - 20, 0.01));

    final menuRect = tester.getRect(
      find.byKey(const Key('quick-actions-menu')),
    );
    final createCard = find.byKey(
      const Key('quick-action-create-channel-card'),
    );
    final dmCard = find.byKey(const Key('quick-action-new-dm-card'));
    final createRect = tester.getRect(createCard);
    final dmRect = tester.getRect(dmCard);

    expect(createRect.left - menuRect.left, closeTo(8, 0.01));
    expect(menuRect.right - createRect.right, closeTo(8, 0.01));
    expect(dmRect.left - menuRect.left, closeTo(8, 0.01));
    expect(menuRect.right - dmRect.right, closeTo(8, 0.01));
    expect(dmRect.top - createRect.bottom, closeTo(8, 0.01));
    expect(dmRect.width, createRect.width);
    expect(dmRect.width, closeTo(menuRect.width - 16, 0.01));

    final cardScheme = Theme.of(tester.element(createCard)).colorScheme;
    final expectedCardColor = Color.alphaBlend(
      cardScheme.onPrimary.withValues(alpha: 0.1),
      cardScheme.primary,
    );
    final createMaterial = tester.widget<Material>(
      find.descendant(of: createCard, matching: find.byType(Material)).first,
    );
    final dmMaterial = tester.widget<Material>(
      find.descendant(of: dmCard, matching: find.byType(Material)).first,
    );
    expect(createMaterial.color, expectedCardColor);
    expect(dmMaterial.color, expectedCardColor);
    expect(
      (createMaterial.borderRadius as BorderRadius).topLeft.x,
      closeTo(12, 0.01),
    );
    expect(
      (dmMaterial.borderRadius as BorderRadius).topLeft.x,
      closeTo(12, 0.01),
    );

    expect(find.text('Start a new stream channel'), findsNothing);
    expect(
      tester.widget<Text>(find.text('Create channel')).style?.fontSize,
      16,
    );
    expect(
      tester.widget<Text>(find.text('New direct message')).style?.fontSize,
      16,
    );
    expect(find.text('Message one or more people'), findsNothing);
  });

  testWidgets('create channel sheet lists type and visibility radio options', (
    tester,
  ) async {
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(testChannels)),
        ],
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('Create or start conversation'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('Create channel'));
    await tester.pumpAndSettle();
    final createSheetRect = tester.getRect(find.byType(BottomSheet).last);
    expect(createSheetRect.top, greaterThanOrEqualTo(24));
    tester.testTextInput.hide();
    await tester.pumpAndSettle();

    expect(find.text('Create a new channel'), findsOneWidget);
    expect(find.text('Name'), findsOneWidget);
    expect(find.text('Description  Optional'), findsOneWidget);
    expect(find.text('Channel type'), findsOneWidget);
    expect(find.text('Ongoing'), findsOneWidget);
    expect(find.text('Temporary'), findsOneWidget);
    expect(find.text('Visibility'), findsOneWidget);
    expect(find.text('Public'), findsOneWidget);
    expect(find.text('Private'), findsOneWidget);
    expect(find.byKey(const Key('create-channel-submit')), findsNothing);
    final nameField = tester.widget<TextField>(
      find.byKey(const Key('create-channel-name')),
    );
    final descriptionField = tester.widget<TextField>(
      find.byKey(const Key('create-channel-description')),
    );
    expect(nameField.textInputAction, TextInputAction.done);
    expect(nameField.onSubmitted, isNotNull);
    expect(descriptionField.textInputAction, TextInputAction.done);
    expect(descriptionField.onSubmitted, isNotNull);
    expect(
      tester.getSize(find.byKey(const Key('create-channel-name'))).height,
      greaterThanOrEqualTo(52),
    );
    expect(
      tester
          .getSize(find.byKey(const Key('create-channel-type-ongoing')))
          .height,
      greaterThanOrEqualTo(56),
    );

    await tester.tap(find.byKey(const Key('create-channel-type-temporary')));
    await tester.pumpAndSettle();

    expect(find.text('Expires after'), findsOneWidget);
    expect(find.text('7 days'), findsOneWidget);
    expect(
      tester.getSize(find.byKey(const Key('create-channel-ttl'))).height,
      greaterThanOrEqualTo(51),
    );

    final privateVisibility = find.byKey(
      const Key('create-channel-visibility-private'),
    );
    await tester.ensureVisible(privateVisibility);
    await tester.pumpAndSettle();
    await tester.tap(privateVisibility);
    await tester.pumpAndSettle();
    expect(
      tester
          .widget<RadioGroup<String>>(find.byType(RadioGroup<String>))
          .groupValue,
      'private',
    );
  });

  testWidgets('new message sheet lists and selects relay members', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);

    const directoryUsers = [
      DirectoryUser(
        pubkey: 'alice',
        displayName: 'Alice',
        nip05Handle: 'alice@example.com',
      ),
      DirectoryUser(pubkey: 'bob', displayName: 'Bob'),
      DirectoryUser(pubkey: 'charlie', displayName: 'Charlie'),
      DirectoryUser(pubkey: 'danielle', displayName: 'Danielle'),
    ];
    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(testChannels)),
          relayDirectoryUsersProvider.overrideWith(
            (ref) async => directoryUsers,
          ),
          relayDirectorySearchProvider.overrideWith((ref, query) async {
            return directoryUsers
                .where(
                  (user) =>
                      user.label.toLowerCase().contains(query.toLowerCase()),
                )
                .toList();
          }),
        ],
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('Create or start conversation'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('New direct message'));
    await tester.pumpAndSettle();

    final dmSheetRect = tester.getRect(find.byType(BottomSheet).last);
    expect(dmSheetRect.top, greaterThanOrEqualTo(24));
    expect(find.text('New message'), findsOneWidget);
    expect(find.text('To:'), findsOneWidget);
    expect(find.text('Search for a person'), findsOneWidget);
    final recipientField = find.byKey(const Key('new-dm-recipient-field'));
    final initialRecipientWidth = tester.getSize(recipientField).width;
    expect(tester.getSize(recipientField).height, greaterThanOrEqualTo(48));
    expect(find.byKey(const Key('new-dm-person-alice')), findsOneWidget);
    expect(find.byKey(const Key('new-dm-person-bob')), findsOneWidget);
    expect(
      find.descendant(
        of: find.byKey(const Key('new-dm-person-alice')),
        matching: find.byType(AvatarImage),
      ),
      findsOneWidget,
    );

    await tester.enterText(find.byKey(const Key('new-dm-search')), 'bob');
    await tester.pump(const Duration(milliseconds: 300));
    await tester.pumpAndSettle();
    expect(find.byKey(const Key('new-dm-person-alice')), findsNothing);
    expect(find.byKey(const Key('new-dm-person-bob')), findsOneWidget);

    await tester.enterText(find.byKey(const Key('new-dm-search')), '');
    await tester.pump(const Duration(milliseconds: 300));
    await tester.pumpAndSettle();
    await tester.tap(find.byKey(const Key('new-dm-person-alice')));
    await tester.pump();

    final aliceChip = find.byKey(const Key('new-dm-selected-alice'));
    final inlineSearch = find.byKey(const Key('new-dm-search'));
    final aliceRect = tester.getRect(aliceChip);
    final inlineSearchRect = tester.getRect(inlineSearch);
    expect(inlineSearchRect.left, greaterThan(aliceRect.right));
    expect(
      (inlineSearchRect.center.dy - aliceRect.center.dy).abs(),
      lessThan(1),
    );
    expect(find.text('Search for a person'), findsNothing);
    expect(tester.widget<TextField>(inlineSearch).focusNode?.hasFocus, isTrue);
    expect(tester.getSize(recipientField).width, initialRecipientWidth);
    expect(tester.getSize(aliceChip).height, 40);
    expect(tester.getSize(aliceChip).width, lessThanOrEqualTo(224));
    final aliceChipMaterial = tester.widget<Material>(
      find.descendant(of: aliceChip, matching: find.byType(Material)).first,
    );
    expect(aliceChipMaterial.shape, isA<StadiumBorder>());
    expect(
      aliceChipMaterial.color,
      Theme.of(tester.element(aliceChip)).colorScheme.surfaceContainerHighest,
    );
    expect(
      tester
          .widget<AvatarImage>(
            find.descendant(of: aliceChip, matching: find.byType(AvatarImage)),
          )
          .radius,
      16,
    );
    expect(
      tester
          .widget<Text>(
            find.descendant(of: aliceChip, matching: find.text('Alice')),
          )
          .style
          ?.fontSize,
      16,
    );
    expect(
      find.descendant(of: aliceChip, matching: find.byIcon(LucideIcons.x)),
      findsNothing,
    );
    expect(find.bySemanticsLabel('Remove Alice'), findsOneWidget);

    await tester.tap(find.byKey(const Key('new-dm-person-bob')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('new-dm-person-charlie')));
    await tester.pump();
    await tester.tap(find.byKey(const Key('new-dm-person-danielle')));
    await tester.pump();

    expect(aliceChip, findsOneWidget);
    expect(find.byKey(const Key('new-dm-selected-bob')), findsOneWidget);
    expect(find.byKey(const Key('new-dm-selected-charlie')), findsOneWidget);
    final danielleChip = find.byKey(const Key('new-dm-selected-danielle'));
    expect(danielleChip, findsOneWidget);
    expect(tester.getSize(recipientField).width, initialRecipientWidth);
    expect(
      tester.getRect(danielleChip).top,
      greaterThan(tester.getRect(aliceChip).top),
    );
    final recipientScrollViews = tester.widgetList<SingleChildScrollView>(
      find.ancestor(
        of: aliceChip,
        matching: find.byType(SingleChildScrollView),
      ),
    );
    expect(
      recipientScrollViews.every(
        (scrollView) => scrollView.scrollDirection == Axis.vertical,
      ),
      isTrue,
    );
    expect(
      find.ancestor(
        of: aliceChip,
        matching: find.byKey(const Key('new-dm-recipient-wrap')),
      ),
      findsOneWidget,
    );
    await tester.tap(find.byKey(const Key('new-dm-selected-bob')));
    await tester.pump();
    expect(find.byKey(const Key('new-dm-selected-bob')), findsNothing);
    expect(find.byKey(const Key('new-dm-person-bob')), findsOneWidget);
    expect(tester.widget<TextField>(inlineSearch).focusNode?.hasFocus, isTrue);
    expect(find.text('Cancel'), findsNothing);
    expect(find.text('Open DM'), findsNothing);
    final searchField = tester.widget<TextField>(
      find.byKey(const Key('new-dm-search')),
    );
    expect(searchField.textInputAction, TextInputAction.done);
    expect(searchField.onSubmitted, isNotNull);
  });

  testWidgets('wraps many recipients without overflowing above the keyboard', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);

    final directoryUsers = [
      for (var index = 0; index < 8; index++)
        DirectoryUser(
          pubkey: 'person-$index',
          displayName: 'Long recipient name number $index',
        ),
    ];

    await tester.pumpWidget(
      buildTestable(
        keyboardInset: 300,
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(testChannels)),
          relayDirectoryUsersProvider.overrideWith(
            (ref) async => directoryUsers,
          ),
        ],
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('Create or start conversation'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('New direct message'));
    await tester.pumpAndSettle();

    for (final user in directoryUsers) {
      final result = find.byKey(Key('new-dm-person-${user.pubkey}'));
      await tester.ensureVisible(result);
      await tester.pumpAndSettle();
      await tester.tap(result);
      await tester.pumpAndSettle();
    }

    expect(find.byKey(const Key('new-dm-recipient-wrap')), findsOneWidget);
    expect(
      find.bySemanticsLabel(RegExp(r'^Remove Long recipient')),
      findsNWidgets(8),
    );
    expect(
      find.text('DMs support up to nine people, including you.'),
      findsOneWidget,
    );
    expect(tester.takeException(), isNull);
  });

  testWidgets('shows local preview people when the relay is offline', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(390, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.resetPhysicalSize);
    addTearDown(tester.view.resetDevicePixelRatio);

    await tester.pumpWidget(
      buildTestable(
        previewDirectory: true,
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(testChannels)),
        ],
      ),
    );
    await tester.pumpAndSettle();

    await tester.tap(find.byTooltip('Create or start conversation'));
    await tester.pumpAndSettle();
    await tester.tap(find.text('New direct message'));
    await tester.pumpAndSettle();

    expect(find.text('Could not load people from this relay.'), findsNothing);
    expect(find.text('Maya Chen'), findsOneWidget);
    expect(find.text('Jordan Brooks'), findsOneWidget);
    expect(find.text('Priya Shah'), findsOneWidget);

    await tester.tap(
      find.byKey(
        const Key(
          'new-dm-person-'
          '1111111111111111111111111111111111111111111111111111111111111111',
        ),
      ),
    );
    await tester.pump();
    expect(find.text('Maya Chen'), findsWidgets);

    await tester.testTextInput.receiveAction(TextInputAction.done);
    await tester.pump();
    expect(
      find.text('Preview only — mock people cannot be messaged.'),
      findsOneWidget,
    );
  });

  testWidgets('hides unjoined and archived channels from the main list', (
    tester,
  ) async {
    final channels = [
      ...testChannels,
      Channel(
        id: '4',
        name: 'open-stream',
        channelType: 'stream',
        visibility: 'open',
        description: 'Available to join',
        createdBy: 'abc',
        createdAt: DateTime(2025),
        memberCount: 8,
        isMember: false,
      ),
      Channel(
        id: '5',
        name: 'archived-stream',
        channelType: 'stream',
        visibility: 'open',
        description: 'Archived channel',
        createdBy: 'abc',
        createdAt: DateTime(2025),
        memberCount: 4,
        isMember: true,
        archivedAt: DateTime(2025, 1, 2),
      ),
    ];

    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(channels)),
        ],
      ),
    );
    await tester.pumpAndSettle();

    // Unjoined and archived channels should not appear in the main list.
    expect(find.text('general'), findsOneWidget);
    expect(find.text('open-stream'), findsNothing);
    expect(find.text('archived-stream'), findsNothing);
  });

  testWidgets('shows empty state when no channels', (tester) async {
    await tester.pumpWidget(
      buildTestable(
        overrides: [channelsProvider.overrideWith(() => _FakeNotifier([]))],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.text('No conversations yet'), findsOneWidget);
  });

  testWidgets('shows error view with retry button', (tester) async {
    await tester.pumpWidget(
      buildTestable(
        overrides: [channelsProvider.overrideWith(() => _ErrorNotifier())],
      ),
    );
    // The error view is gated on a grace timer in ChannelsPage to absorb
    // transient AsyncError frames during relay reconnect. Pump once to mount
    // and schedule the timer, advance the fake clock past the grace window,
    // then pump again to flush the setState the timer triggered.
    await tester.pump();
    await tester.pump(const Duration(seconds: 3));
    await tester.pump();

    expect(find.text('Could not load channels'), findsOneWidget);
    expect(find.text('Retry'), findsOneWidget);
  });

  testWidgets('renders and clears unread dot indicator', (tester) async {
    final channels = [
      Channel(
        id: '1',
        name: 'general',
        channelType: 'stream',
        visibility: 'open',
        description: 'General discussion',
        createdBy: 'abc',
        createdAt: DateTime(2025),
        memberCount: 10,
        lastMessageAt: DateTime.fromMillisecondsSinceEpoch(
          20 * 1000,
          isUtc: true,
        ),
        isMember: true,
      ),
    ];
    final readState = _FakeReadStateNotifier(
      const ReadStateState(
        isReady: true,
        pubkey: 'pk',
        contexts: {'1': 10},
        version: 0,
      ),
    );

    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(
            () => _FakeNotifier(
              channels,
              observedEventsByChannel: {
                '1': [_observed(id: 'msg-1', createdAt: 20)],
              },
            ),
          ),
          readStateProvider.overrideWith(() => readState),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('channel-unread-dot-1')), findsOneWidget);

    readState.markContextRead('1', 20);
    await tester.pump();

    expect(find.byKey(const Key('channel-unread-dot-1')), findsNothing);
  });

  testWidgets('renders numeric unread count for counted events', (
    tester,
  ) async {
    final channels = [
      Channel(
        id: '1',
        name: 'general',
        channelType: 'stream',
        visibility: 'open',
        description: 'General discussion',
        createdBy: 'abc',
        createdAt: DateTime(2025),
        memberCount: 10,
        lastMessageAt: DateTime.fromMillisecondsSinceEpoch(
          30 * 1000,
          isUtc: true,
        ),
        isMember: true,
      ),
    ];
    final readState = _FakeReadStateNotifier(
      const ReadStateState(
        isReady: true,
        pubkey: 'pk',
        contexts: {'1': 10},
        version: 0,
      ),
    );

    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(
            () => _FakeNotifier(
              channels,
              observedEventsByChannel: {
                '1': [
                  _observed(
                    id: 'reply-1',
                    createdAt: 20,
                    rootId: 'root',
                    isThreadedReply: true,
                  ),
                  _observed(
                    id: 'reply-2',
                    createdAt: 30,
                    rootId: 'root',
                    isThreadedReply: true,
                  ),
                ],
              },
            ),
          ),
          readStateProvider.overrideWith(() => readState),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(find.byKey(const Key('channel-unread-1')), findsOneWidget);
    expect(find.text('2'), findsOneWidget);
    expect(find.byKey(const Key('channel-unread-dot-1')), findsNothing);
  });

  testWidgets('seeds first loaded channels as read', (tester) async {
    final channels = [
      Channel(
        id: '1',
        name: 'general',
        channelType: 'stream',
        visibility: 'open',
        description: 'General discussion',
        createdBy: 'abc',
        createdAt: DateTime(2025),
        memberCount: 10,
        lastMessageAt: DateTime.fromMillisecondsSinceEpoch(
          20 * 1000,
          isUtc: true,
        ),
        isMember: true,
      ),
    ];
    final readState = _FakeReadStateNotifier(
      const ReadStateState(
        isReady: true,
        pubkey: 'pk',
        contexts: {},
        version: 0,
      ),
    );

    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(channels)),
          readStateProvider.overrideWith(() => readState),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(readState.seededContexts, {'1': 20});
    expect(readState.markedContexts, isEmpty);
    expect(find.byKey(const Key('channel-unread-1')), findsNothing);
  });

  testWidgets('waits for read-state readiness before initial seeding', (
    tester,
  ) async {
    final channels = [
      Channel(
        id: '1',
        name: 'general',
        channelType: 'stream',
        visibility: 'open',
        description: 'General discussion',
        createdBy: 'abc',
        createdAt: DateTime(2025),
        memberCount: 10,
        lastMessageAt: DateTime.fromMillisecondsSinceEpoch(
          20 * 1000,
          isUtc: true,
        ),
        isMember: true,
      ),
    ];
    final readState = _FakeReadStateNotifier(
      const ReadStateState(
        isReady: false,
        pubkey: 'pk',
        contexts: {},
        version: 0,
      ),
    );

    await tester.pumpWidget(
      buildTestable(
        overrides: [
          channelsProvider.overrideWith(() => _FakeNotifier(channels)),
          readStateProvider.overrideWith(() => readState),
        ],
      ),
    );
    await tester.pumpAndSettle();

    expect(readState.seededContexts, isEmpty);
    expect(readState.markedContexts, isEmpty);

    readState.setReady();
    await tester.pumpAndSettle();

    expect(readState.seededContexts, {'1': 20});
    expect(readState.markedContexts, isEmpty);
  });
}

Widget _buildSettingsPage(BuildContext context) =>
    const Scaffold(body: Text('Injected settings'));

class _FakeNotifier extends ChannelsNotifier {
  final List<Channel> _channels;
  final Map<String, Map<String, ObservedUnreadEvent>> _observedEventsByChannel;

  _FakeNotifier(
    this._channels, {
    Map<String, List<ObservedUnreadEvent>> observedEventsByChannel = const {},
  }) : _observedEventsByChannel = {
         for (final entry in observedEventsByChannel.entries)
           entry.key: {for (final event in entry.value) event.id: event},
       };

  @override
  Future<List<Channel>> build() async => _channels;

  @override
  Map<String, int> get latestObservedByChannel => {
    for (final entry in _observedEventsByChannel.entries)
      if (entry.value.isNotEmpty)
        entry.key: entry.value.values
            .map((event) => event.createdAt)
            .reduce((left, right) => left > right ? left : right),
  };

  @override
  Map<String, Map<String, ObservedUnreadEvent>>
  get observedUnreadEventsByChannel => _observedEventsByChannel;
}

class _FakeCommunityListNotifier extends CommunityListNotifier {
  _FakeCommunityListNotifier(this._communities);

  List<Community> _communities;
  final List<String> removedIds = [];

  @override
  Future<List<Community>> build() async => _communities;

  @override
  Future<void> removeCommunity(String id) async {
    removedIds.add(id);
    _communities = _communities
        .where((community) => community.id != id)
        .toList();
    state = AsyncData(_communities);
  }
}

class _ErrorNotifier extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() => Future.error('Connection refused');
}

class _LoadingNotifier extends ChannelsNotifier {
  @override
  Future<List<Channel>> build() => Completer<List<Channel>>().future;
}

class _ReconnectingRelaySession extends RelaySessionNotifier {
  final SessionStatus initialStatus;

  _ReconnectingRelaySession({this.initialStatus = SessionStatus.reconnecting});

  @override
  SessionState build() => SessionState(status: initialStatus);

  @override
  Future<List<NostrEvent>> fetchHistory(
    NostrFilter filter, {
    Duration timeout = const Duration(seconds: 8),
  }) async => [];

  @override
  Future<void Function()> subscribe(
    NostrFilter filter,
    void Function(NostrEvent) onEvent, {
    void Function(String message)? onClosed,
  }) async => () {};

  void connect() {
    state = const SessionState(status: SessionStatus.connected);
  }

  void setReconnecting() {
    state = const SessionState(status: SessionStatus.reconnecting);
  }
}

class _FakeProfileNotifier extends ProfileNotifier {
  @override
  Future<UserProfile?> build() async =>
      const UserProfile(pubkey: 'aabb', displayName: 'Test');
}

class _FakePresenceNotifier extends PresenceNotifier {
  @override
  Future<String> build() async => 'online';
}

class _FakeReadStateNotifier extends ReadStateNotifier {
  final ReadStateState _initialState;
  final Map<String, int> seededContexts = {};
  final Map<String, int> markedContexts = {};

  _FakeReadStateNotifier(this._initialState);

  @override
  ReadStateState build() => _initialState;

  void setReady() {
    state = ReadStateState(
      isReady: true,
      pubkey: state.pubkey,
      contexts: state.contexts,
      version: state.version + 1,
      forcedUnreadContexts: state.forcedUnreadContexts,
    );
  }

  @override
  void seedContextRead(String contextId, int unixTimestamp) {
    seededContexts[contextId] = unixTimestamp;
    state = state.copyWithContext(contextId, unixTimestamp);
  }

  @override
  void markContextRead(
    String contextId,
    int unixTimestamp, {
    bool clearForcedMessages = false,
  }) {
    markedContexts[contextId] = unixTimestamp;
    state = state.copyWithContext(contextId, unixTimestamp);
  }
}

ObservedUnreadEvent _observed({
  required String id,
  required int createdAt,
  String? rootId,
  bool highPriority = false,
  bool isThreadedReply = false,
}) => makeObservedUnreadEvent(
  id: id,
  createdAt: createdAt,
  rootId: rootId,
  highPriority: highPriority,
  channelType: 'stream',
  isThreadedReply: isThreadedReply,
);
