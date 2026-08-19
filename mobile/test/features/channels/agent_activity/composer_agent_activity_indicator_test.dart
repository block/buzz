import 'package:flutter/cupertino.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:buzz/features/channels/agent_activity/active_agent_turns.dart';
import 'package:buzz/features/channels/agent_activity/composer_agent_activity_indicator.dart';
import 'package:buzz/features/channels/agent_activity/observer_models.dart';
import 'package:buzz/features/channels/agent_activity/observer_subscription.dart';
import 'package:buzz/features/channels/agent_activity/working_bots_provider.dart';
import 'package:buzz/features/channels/channel_typing_provider.dart';
import 'package:buzz/shared/profile/user_cache_provider.dart';
import 'package:buzz/shared/profile/user_profile.dart';
import 'package:buzz/shared/theme/theme.dart';

const _channelId = 'channel-1';
const _agentPubkey = 'agent-a';
const _turnId = 'turn-a';
const _secondAgentPubkey = 'agent-b';
const _secondTurnId = 'turn-b';
const _scope = (channelId: _channelId, threadHeadId: null);
const _turnKey = (
  channelId: _channelId,
  agentPubkey: _agentPubkey,
  turnId: _turnId,
);
const _secondTurnKey = (
  channelId: _channelId,
  agentPubkey: _secondAgentPubkey,
  turnId: _secondTurnId,
);

void main() {
  testWidgets(
    'expands inline, keeps human typing separate, and preserves completion',
    (tester) async {
      final container = ProviderContainer(
        overrides: [
          composerActivityStateProvider(
            _scope,
          ).overrideWith((ref) => ref.watch(_testComposerActivityProvider)),
          agentTurnStatesProvider.overrideWith(
            (ref) => ref.watch(_testTurnStatesProvider),
          ),
          observerTurnSubscriptionProvider(
            _turnKey,
          ).overrideWithValue(_observerState),
          userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
        ],
      );
      addTearDown(container.dispose);
      container
          .read(_testComposerActivityProvider.notifier)
          .setState(
            const ComposerActivityState(
              agents: [
                WorkingAgentSignal(
                  pubkey: _agentPubkey,
                  source: AgentWorkingSource.observer,
                  canViewActivity: true,
                  turnId: _turnId,
                ),
              ],
              humanTyping: [
                TypingEntry(pubkey: 'human-a', expiresAtMs: 9999999999999),
              ],
            ),
          );
      container.read(_testTurnStatesProvider.notifier).setStates([
        _turn(AgentTurnPhase.working),
      ]);

      await tester.pumpWidget(_app(container));
      await tester.pump();

      expect(
        find.byKey(const ValueKey('composer-agent-activity-control')),
        findsOneWidget,
      );
      expect(
        find.byKey(const ValueKey('channel-typing-indicator')),
        findsOneWidget,
      );
      expect(find.byType(BottomSheet), findsNothing);

      await tester.tap(
        find.byKey(const ValueKey('composer-agent-activity-control')),
      );
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 240));

      final panel = find.byKey(const ValueKey('composer-agent-activity-panel'));
      final control = find.byKey(
        const ValueKey('composer-agent-activity-control'),
      );
      final collapse = find.byKey(
        const ValueKey('composer-agent-activity-collapse'),
      );
      final composer = find.byKey(const ValueKey('fake-composer'));
      expect(panel, findsOneWidget);
      expect(control, findsNothing);
      expect(
        find.byKey(const ValueKey('composer-agent-activity-surface')),
        findsOneWidget,
      );
      expect(find.byType(BottomSheet), findsNothing);
      expect(
        tester.getRect(panel).bottom,
        lessThanOrEqualTo(tester.getRect(composer).top),
      );
      expect(
        tester.getRect(collapse).bottom,
        closeTo(tester.getRect(panel).bottom, 0.1),
      );
      expect(find.text('Live activity may be partial.'), findsOneWidget);
      expect(
        find.byKey(const ValueKey('composer-agent-segmented-container')),
        findsNothing,
      );
      expect(find.text('Pollen'), findsNothing);
      expect(find.text('Thinking'), findsOneWidget);
      expect(find.text('private chain of thought'), findsNothing);
      expect(find.textContaining('/private/path'), findsNothing);

      await tester.tap(collapse);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 240));
      await tester.pump();
      expect(panel, findsNothing);
      expect(control, findsOneWidget);

      await tester.tap(control);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 240));
      expect(panel, findsOneWidget);
      expect(control, findsNothing);

      container
          .read(_testComposerActivityProvider.notifier)
          .setState(const ComposerActivityState(agents: [], humanTyping: []));
      container.read(_testTurnStatesProvider.notifier).setStates([
        _turn(AgentTurnPhase.finished),
      ]);
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 240));

      expect(panel, findsOneWidget);
      expect(find.text('Finished'), findsWidgets);

      await tester.tap(
        find.byKey(const ValueKey('composer-agent-activity-collapse')),
      );
      await tester.pump();
      await tester.pump(const Duration(milliseconds: 240));
      await tester.pump();
      expect(panel, findsNothing);
      expect(control, findsNothing);
    },
  );

  testWidgets('keeps a collapsed terminal error openable', (tester) async {
    final container = ProviderContainer(
      overrides: [
        composerActivityStateProvider(_scope).overrideWithValue(
          const ComposerActivityState(
            agents: [
              WorkingAgentSignal(
                pubkey: _agentPubkey,
                source: AgentWorkingSource.observer,
                canViewActivity: true,
                turnId: _turnId,
              ),
            ],
            humanTyping: [],
          ),
        ),
        agentTurnStatesProvider.overrideWithValue([
          _turn(AgentTurnPhase.error),
        ]),
        observerTurnSubscriptionProvider(
          _turnKey,
        ).overrideWithValue(_observerState),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(_app(container));
    await tester.pump();

    final control = find.byKey(
      const ValueKey('composer-agent-activity-control'),
    );
    expect(control, findsOneWidget);
    expect(find.text('Pollen stopped with an error'), findsOneWidget);

    await tester.tap(control);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 240));

    expect(
      find.byKey(const ValueKey('composer-agent-activity-panel')),
      findsOneWidget,
    );
    expect(find.text('Error'), findsOneWidget);
    expect(find.text('Thinking'), findsOneWidget);
  });

  testWidgets('labels a cancelled turn separately from a finished turn', (
    tester,
  ) async {
    final container = ProviderContainer(
      overrides: [
        composerActivityStateProvider(_scope).overrideWithValue(
          const ComposerActivityState(
            agents: [
              WorkingAgentSignal(
                pubkey: _agentPubkey,
                source: AgentWorkingSource.observer,
                canViewActivity: true,
                isWorking: false,
                phase: AgentTurnPhase.cancelled,
                turnId: _turnId,
              ),
            ],
            humanTyping: [],
          ),
        ),
        agentTurnStatesProvider.overrideWithValue([
          _turn(AgentTurnPhase.cancelled),
        ]),
        observerTurnSubscriptionProvider(
          _turnKey,
        ).overrideWithValue(_observerState),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(_app(container));
    await tester.pump();

    final control = find.byKey(
      const ValueKey('composer-agent-activity-control'),
    );
    expect(control, findsOneWidget);
    expect(find.text('Pollen was cancelled'), findsOneWidget);

    await tester.tap(control);
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 240));

    expect(find.text('Cancelled'), findsOneWidget);
  });

  testWidgets('summarizes mixed working and terminal agent states', (
    tester,
  ) async {
    final container = ProviderContainer(
      overrides: [
        composerActivityStateProvider(_scope).overrideWithValue(
          const ComposerActivityState(
            agents: [
              WorkingAgentSignal(
                pubkey: _agentPubkey,
                source: AgentWorkingSource.observer,
                canViewActivity: true,
                phase: AgentTurnPhase.working,
                turnId: _turnId,
              ),
              WorkingAgentSignal(
                pubkey: _secondAgentPubkey,
                source: AgentWorkingSource.observer,
                canViewActivity: true,
                isWorking: false,
                phase: AgentTurnPhase.error,
                turnId: _secondTurnId,
              ),
            ],
            humanTyping: [],
          ),
        ),
        agentTurnStatesProvider.overrideWithValue([
          _turn(AgentTurnPhase.working),
          _turnFor(
            pubkey: _secondAgentPubkey,
            turnId: _secondTurnId,
            phase: AgentTurnPhase.error,
          ),
        ]),
        observerTurnSubscriptionProvider(
          _turnKey,
        ).overrideWithValue(_observerState),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(_app(container));
    await tester.pump();

    expect(find.text('1 working · 1 error'), findsOneWidget);
    expect(find.text('2 agents are working…'), findsNothing);
  });

  testWidgets('summarizes multiple retained terminal outcomes', (tester) async {
    final container = ProviderContainer(
      overrides: [
        composerActivityStateProvider(_scope).overrideWithValue(
          const ComposerActivityState(
            agents: [
              WorkingAgentSignal(
                pubkey: _agentPubkey,
                source: AgentWorkingSource.observer,
                canViewActivity: true,
                isWorking: false,
                phase: AgentTurnPhase.finished,
                turnId: _turnId,
              ),
              WorkingAgentSignal(
                pubkey: _secondAgentPubkey,
                source: AgentWorkingSource.observer,
                canViewActivity: true,
                isWorking: false,
                phase: AgentTurnPhase.error,
                turnId: _secondTurnId,
              ),
            ],
            humanTyping: [],
          ),
        ),
        agentTurnStatesProvider.overrideWithValue([
          _turn(AgentTurnPhase.finished),
          _turnFor(
            pubkey: _secondAgentPubkey,
            turnId: _secondTurnId,
            phase: AgentTurnPhase.error,
          ),
        ]),
        observerTurnSubscriptionProvider(
          _turnKey,
        ).overrideWithValue(_observerState),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(_app(container));
    await tester.pump();

    expect(find.text('1 finished · 1 error'), findsOneWidget);
    expect(find.text('2 agents are working…'), findsNothing);
  });

  testWidgets('reduced motion makes inline size changes immediate', (
    tester,
  ) async {
    final container = ProviderContainer(
      overrides: [
        composerActivityStateProvider(_scope).overrideWithValue(
          const ComposerActivityState(
            agents: [
              WorkingAgentSignal(
                pubkey: _agentPubkey,
                source: AgentWorkingSource.observer,
                canViewActivity: true,
                turnId: _turnId,
              ),
            ],
            humanTyping: [],
          ),
        ),
        agentTurnStatesProvider.overrideWithValue([
          _turn(AgentTurnPhase.working),
        ]),
        observerTurnSubscriptionProvider(
          _turnKey,
        ).overrideWithValue(_observerState),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
      ],
    );
    addTearDown(container.dispose);

    tester.view.physicalSize = const Size(400, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    await tester.pumpWidget(
      _app(container, disableAnimations: true, compactWidthFactor: 0.85),
    );
    await tester.pump();

    expect(
      find.descendant(
        of: find.byType(ComposerAgentActivityIndicator),
        matching: find.byType(AnimatedSize),
      ),
      findsNothing,
    );
    final compactWidth = tester
        .getSize(find.byKey(const ValueKey('composer-agent-activity-surface')))
        .width;
    await tester.tap(
      find.byKey(const ValueKey('composer-agent-activity-control')),
    );
    await tester.pump();
    await tester.pump();
    expect(
      tester
          .getSize(
            find.byKey(const ValueKey('composer-agent-activity-overlay')),
          )
          .width,
      closeTo(376, 0.1),
    );
    expect(
      tester
          .getSize(
            find.byKey(const ValueKey('composer-agent-activity-overlay')),
          )
          .width,
      greaterThan(compactWidth),
    );
  });

  testWidgets('width and height morph together and reverse mid-expansion', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(400, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final container = ProviderContainer(
      overrides: [
        composerActivityStateProvider(_scope).overrideWithValue(
          const ComposerActivityState(
            agents: [
              WorkingAgentSignal(
                pubkey: _agentPubkey,
                source: AgentWorkingSource.observer,
                canViewActivity: true,
                turnId: _turnId,
              ),
            ],
            humanTyping: [],
          ),
        ),
        agentTurnStatesProvider.overrideWithValue([
          _turn(AgentTurnPhase.working),
        ]),
        observerTurnSubscriptionProvider(
          _turnKey,
        ).overrideWithValue(_observerState),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(_app(container, compactWidthFactor: 0.85));
    await tester.pump();
    final compactWidth = tester
        .getSize(find.byKey(const ValueKey('composer-agent-activity-surface')))
        .width;
    await tester.tap(
      find.byKey(const ValueKey('composer-agent-activity-control')),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 60));

    final overlay = find.byKey(
      const ValueKey('composer-agent-activity-overlay'),
    );
    final panel = find.byKey(const ValueKey('composer-agent-activity-panel'));
    final expandingSize = tester.getSize(overlay);
    final expandingPanelSize = tester.getSize(panel);
    expect(tester.getCenter(overlay).dx, closeTo(200, 0.1));
    expect(expandingSize.width, greaterThan(compactWidth));
    expect(expandingSize.width, lessThan(376));
    expect(expandingSize.height, greaterThan(52));
    expect(expandingSize.height, lessThan(328));
    expect(
      expandingPanelSize.width,
      closeTo(expandingSize.width - Grid.twelve * 2, 0.1),
    );
    expect(expandingPanelSize.height, greaterThan(44));
    expect(expandingPanelSize.height, lessThan(312));

    tester
        .widget<InkWell>(
          find.byKey(const ValueKey('composer-agent-activity-collapse')),
        )
        .onTap!();
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 20));
    expect(tester.getCenter(overlay).dx, closeTo(200, 0.1));
    expect(tester.getSize(overlay).width, lessThan(expandingSize.width));
    expect(tester.getSize(overlay).height, lessThan(expandingSize.height));
    expect(tester.getSize(panel).height, lessThan(expandingPanelSize.height));

    final reversingSize = tester.getSize(overlay);
    await tester.tap(
      find.byKey(const ValueKey('composer-agent-activity-collapse')),
    );
    await tester.pump();
    await tester.pump(const Duration(milliseconds: 20));
    expect(tester.getSize(overlay).width, greaterThan(reversingSize.width));
    expect(tester.getSize(overlay).height, greaterThan(reversingSize.height));

    await tester.tap(
      find.byKey(const ValueKey('composer-agent-activity-collapse')),
    );
    await tester.pump();

    await tester.pumpAndSettle();
    expect(overlay, findsNothing);
    expect(
      tester
          .getSize(
            find.byKey(const ValueKey('composer-agent-activity-surface')),
          )
          .width,
      closeTo(compactWidth, 0.1),
    );
  });

  testWidgets('multiple agents use a top segmented selector', (tester) async {
    tester.view.physicalSize = const Size(220, 600);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final container = _multiAgentContainer();
    addTearDown(container.dispose);

    await tester.pumpWidget(_app(container));
    await tester.pump();
    await tester.tap(
      find.byKey(const ValueKey('composer-agent-activity-control')),
    );
    await tester.pumpAndSettle();

    expect(
      find.byKey(const ValueKey('composer-agent-activity-selector')),
      findsOneWidget,
    );
    final selector = find.byKey(
      const ValueKey('composer-agent-segmented-container'),
    );
    expect(selector, findsOneWidget);
    expect(find.text('Pollen'), findsOneWidget);
    expect(find.text('Sprig'), findsOneWidget);
    expect(find.text('Running Search messages'), findsNothing);
    final sprigSegment = find.byKey(
      const ValueKey('composer-agent-segment-agent-b'),
    );
    final selectorScrollView = find.descendant(
      of: find.byKey(const ValueKey('composer-agent-activity-selector')),
      matching: find.byType(Scrollable),
    );
    expect(
      tester
          .state<ScrollableState>(selectorScrollView)
          .position
          .maxScrollExtent,
      greaterThan(0),
    );
    expect(
      tester
          .getSize(find.byKey(const ValueKey('composer-agent-segment-agent-a')))
          .height,
      closeTo(tester.getSize(sprigSegment).height, 0.1),
    );
    expect(
      tester
          .widget<Material>(
            find.byKey(const ValueKey('composer-agent-segment-agent-a')),
          )
          .color,
      AppTheme.light().colorScheme.primary,
    );
    expect(find.bySemanticsLabel('Pollen'), findsOneWidget);
    expect(
      tester.getTopLeft(selector).dy,
      lessThan(
        tester
            .getTopLeft(
              find.byKey(const ValueKey('composer-agent-activity-transcript')),
            )
            .dy,
      ),
    );

    await tester.ensureVisible(sprigSegment);
    await tester.tap(sprigSegment);
    await tester.pumpAndSettle();

    expect(
      tester.widget<Material>(sprigSegment).color,
      AppTheme.light().colorScheme.primary,
    );
    expect(find.text('Sprig'), findsOneWidget);
    expect(find.text('Running Search messages'), findsOneWidget);
    expect(find.text('Running Read file'), findsNothing);
  });

  for (final platform in [TargetPlatform.android, TargetPlatform.iOS]) {
    for (final textScale in [2.0, 3.0]) {
      testWidgets(
        'narrow $platform panel stays operable at ${textScale}x text',
        (tester) async {
          debugDefaultTargetPlatformOverride = platform;
          addTearDown(() => debugDefaultTargetPlatformOverride = null);
          tester.view.physicalSize = const Size(220, 600);
          tester.view.devicePixelRatio = 1;
          addTearDown(tester.view.reset);
          final container = _multiAgentContainer();
          addTearDown(container.dispose);

          await tester.pumpWidget(
            _app(
              container,
              disableAnimations: true,
              textScaler: TextScaler.linear(textScale),
            ),
          );
          await tester.pump();
          expect(tester.takeException(), isNull);

          await tester.tap(
            find.byKey(const ValueKey('composer-agent-activity-control')),
          );
          await tester.pump();

          expect(tester.takeException(), isNull);
          expect(find.text('Live activity'), findsOneWidget);
          expect(find.text('Working'), findsOneWidget);
          expect(
            find.byKey(const ValueKey('composer-agent-activity-collapse')),
            findsOneWidget,
          );

          final sprigSegment = find.byKey(
            const ValueKey('composer-agent-segment-agent-b'),
          );
          await tester.ensureVisible(sprigSegment);
          await tester.tap(sprigSegment);
          await tester.pump();

          expect(tester.takeException(), isNull);
          expect(find.text('Sprig'), findsOneWidget);
          expect(find.text('Running Search messages'), findsOneWidget);

          await tester.tap(
            find.byKey(const ValueKey('composer-agent-activity-collapse')),
          );
          await tester.pump();
          await tester.pump();

          expect(tester.takeException(), isNull);
          expect(
            find.byKey(const ValueKey('composer-agent-activity-panel')),
            findsNothing,
          );
          debugDefaultTargetPlatformOverride = null;
        },
      );
    }
  }

  testWidgets('iOS uses the native sliding segmented control', (tester) async {
    debugDefaultTargetPlatformOverride = TargetPlatform.iOS;
    addTearDown(() => debugDefaultTargetPlatformOverride = null);
    tester.view.physicalSize = const Size(400, 600);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final container = _multiAgentContainer();
    addTearDown(container.dispose);

    await tester.pumpWidget(_app(container));
    await tester.pump();
    await tester.tap(
      find.byKey(const ValueKey('composer-agent-activity-control')),
    );
    await tester.pumpAndSettle();

    final selector = find.byKey(
      const ValueKey('composer-agent-segmented-container'),
    );
    final control = tester.widget<CupertinoSlidingSegmentedControl<String>>(
      selector,
    );
    final frame = find.byKey(const ValueKey('composer-agent-segmented-frame'));
    final frameWidget = tester.widget<ClipRSuperellipse>(frame);
    final panel = find.byKey(const ValueKey('composer-agent-activity-panel'));
    expect(control.groupValue, _agentPubkey);
    expect(control.thumbColor, AppTheme.light().colorScheme.primary);
    expect(
      frameWidget.borderRadius,
      BorderRadius.circular(Radii.dialog - Grid.xxs),
    );
    expect(
      tester.getRect(frame).left,
      closeTo(tester.getRect(panel).left + Grid.xxs, 0.1),
    );
    expect(
      tester.getRect(frame).right,
      closeTo(tester.getRect(panel).right - Grid.xxs, 0.1),
    );
    expect(find.bySemanticsLabel('Pollen'), findsOneWidget);

    final sprigSegment = find.byKey(
      const ValueKey('composer-agent-segment-agent-b'),
    );
    await tester.ensureVisible(sprigSegment);
    await tester.tap(sprigSegment);
    await tester.pumpAndSettle();

    expect(
      tester
          .widget<CupertinoSlidingSegmentedControl<String>>(selector)
          .groupValue,
      _secondAgentPubkey,
    );
    expect(find.text('Running Search messages'), findsOneWidget);
    expect(find.text('Running Read file'), findsNothing);
    debugDefaultTargetPlatformOverride = null;
  });

  testWidgets(
    'overlays upward at the taller target without changing layout height',
    (tester) async {
      tester.view.physicalSize = const Size(400, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final container = ProviderContainer(
        overrides: [
          composerActivityStateProvider(_scope).overrideWithValue(
            const ComposerActivityState(
              agents: [
                WorkingAgentSignal(
                  pubkey: _agentPubkey,
                  source: AgentWorkingSource.observer,
                  canViewActivity: true,
                  turnId: _turnId,
                ),
              ],
              humanTyping: [],
            ),
          ),
          agentTurnStatesProvider.overrideWithValue([
            _turn(AgentTurnPhase.working),
          ]),
          observerTurnSubscriptionProvider(
            _turnKey,
          ).overrideWithValue(_observerState),
          userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
        ],
      );
      addTearDown(container.dispose);

      await tester.pumpWidget(
        _app(container, disableAnimations: true, overlayTopBoundary: 100),
      );
      await tester.pump();
      final indicator = find.byType(ComposerAgentActivityIndicator);
      final compactHeight = tester.getSize(indicator).height;
      final composerTop = tester.getTopLeft(
        find.byKey(const ValueKey('fake-composer')),
      );

      await tester.tap(
        find.byKey(const ValueKey('composer-agent-activity-control')),
      );
      await tester.pump();

      final overlay = find.byKey(
        const ValueKey('composer-agent-activity-overlay'),
      );
      expect(tester.getSize(indicator).height, closeTo(compactHeight, 0.1));
      expect(
        tester.getTopLeft(find.byKey(const ValueKey('fake-composer'))),
        composerTop,
      );
      expect(tester.getSize(overlay).height, closeTo(328, 0.1));
      expect(tester.getRect(overlay).top, greaterThanOrEqualTo(100));
      expect(
        find.byKey(const ValueKey('composer-agent-activity-transcript')),
        findsOneWidget,
      );
    },
  );

  testWidgets('caps the overlay below the supplied top navigation boundary', (
    tester,
  ) async {
    tester.view.physicalSize = const Size(400, 800);
    tester.view.devicePixelRatio = 1;
    addTearDown(tester.view.reset);
    final container = ProviderContainer(
      overrides: [
        composerActivityStateProvider(_scope).overrideWithValue(
          const ComposerActivityState(
            agents: [
              WorkingAgentSignal(
                pubkey: _agentPubkey,
                source: AgentWorkingSource.observer,
                canViewActivity: true,
                turnId: _turnId,
              ),
            ],
            humanTyping: [],
          ),
        ),
        agentTurnStatesProvider.overrideWithValue([
          _turn(AgentTurnPhase.working),
        ]),
        observerTurnSubscriptionProvider(
          _turnKey,
        ).overrideWithValue(_observerState),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(
      _app(container, disableAnimations: true, overlayTopBoundary: 520),
    );
    await tester.pump();
    await tester.tap(
      find.byKey(const ValueKey('composer-agent-activity-control')),
    );
    await tester.pump();

    final overlay = find.byKey(
      const ValueKey('composer-agent-activity-overlay'),
    );
    expect(tester.getRect(overlay).top, greaterThanOrEqualTo(520));
    expect(tester.getSize(overlay).height, lessThan(328));
  });

  testWidgets(
    'dismisses an open keyboard and restores it after collapse with draft intact',
    (tester) async {
      tester.view.physicalSize = const Size(400, 800);
      tester.view.devicePixelRatio = 1;
      addTearDown(tester.view.reset);
      final focusNode = FocusNode();
      final draftController = TextEditingController(text: 'Keep this draft');
      final interactionLock = ValueNotifier(false);
      addTearDown(focusNode.dispose);
      addTearDown(draftController.dispose);
      addTearDown(interactionLock.dispose);
      final container = ProviderContainer(
        overrides: [
          composerActivityStateProvider(_scope).overrideWithValue(
            const ComposerActivityState(
              agents: [
                WorkingAgentSignal(
                  pubkey: _agentPubkey,
                  source: AgentWorkingSource.observer,
                  canViewActivity: true,
                  turnId: _turnId,
                ),
              ],
              humanTyping: [],
            ),
          ),
          agentTurnStatesProvider.overrideWithValue([
            _turn(AgentTurnPhase.working),
          ]),
          observerTurnSubscriptionProvider(
            _turnKey,
          ).overrideWithValue(_observerState),
          userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
        ],
      );
      addTearDown(container.dispose);

      await tester.pumpWidget(
        _app(
          container,
          disableAnimations: true,
          composerFocusNode: focusNode,
          composerInteractionLock: interactionLock,
          draftController: draftController,
          viewInsets: const EdgeInsets.only(bottom: 300),
        ),
      );
      focusNode.requestFocus();
      await tester.pump();
      expect(
        MediaQuery.viewInsetsOf(
          tester.element(find.byType(ComposerAgentActivityIndicator)),
        ).bottom,
        300,
      );

      tester
          .widget<InkWell>(
            find.byKey(const ValueKey('composer-agent-activity-control')),
          )
          .onTap!();
      await tester.pump();
      expect(focusNode.hasFocus, isFalse);
      expect(interactionLock.value, isTrue);
      expect(
        find.byKey(const ValueKey('composer-agent-activity-panel')),
        findsNothing,
      );

      await tester.pumpWidget(
        _app(
          container,
          disableAnimations: true,
          composerFocusNode: focusNode,
          composerInteractionLock: interactionLock,
          draftController: draftController,
        ),
      );
      await tester.pump();
      expect(
        find.byKey(const ValueKey('composer-agent-activity-panel')),
        findsOneWidget,
      );
      expect(focusNode.canRequestFocus, isTrue);

      await tester.tap(
        find.byKey(const ValueKey('composer-agent-activity-collapse')),
      );
      await tester.pump();
      await tester.pump();
      expect(focusNode.canRequestFocus, isTrue);
      expect(focusNode.hasFocus, isTrue);
      expect(interactionLock.value, isFalse);
      expect(draftController.text, 'Keep this draft');
    },
  );

  testWidgets('collapse leaves a previously closed keyboard closed', (
    tester,
  ) async {
    final focusNode = FocusNode();
    final interactionLock = ValueNotifier(false);
    addTearDown(focusNode.dispose);
    addTearDown(interactionLock.dispose);
    final container = ProviderContainer(
      overrides: [
        composerActivityStateProvider(_scope).overrideWithValue(
          const ComposerActivityState(
            agents: [
              WorkingAgentSignal(
                pubkey: _agentPubkey,
                source: AgentWorkingSource.observer,
                canViewActivity: true,
                turnId: _turnId,
              ),
            ],
            humanTyping: [],
          ),
        ),
        agentTurnStatesProvider.overrideWithValue([
          _turn(AgentTurnPhase.working),
        ]),
        observerTurnSubscriptionProvider(
          _turnKey,
        ).overrideWithValue(_observerState),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(
      _app(
        container,
        disableAnimations: true,
        composerFocusNode: focusNode,
        composerInteractionLock: interactionLock,
      ),
    );
    await tester.pump();
    await tester.tap(
      find.byKey(const ValueKey('composer-agent-activity-control')),
    );
    await tester.pump();
    expect(focusNode.canRequestFocus, isTrue);
    expect(interactionLock.value, isTrue);
    await tester.tap(
      find.byKey(const ValueKey('composer-agent-activity-collapse')),
    );
    await tester.pump();
    await tester.pump();

    expect(focusNode.canRequestFocus, isTrue);
    expect(focusNode.hasFocus, isFalse);
    expect(interactionLock.value, isFalse);
  });

  testWidgets(
    'composer activation hands off from activity before restoring focus',
    (tester) async {
      final focusNode = FocusNode();
      final interactionLock = ValueNotifier(false);
      final activationRequests = ValueNotifier(0);
      addTearDown(focusNode.dispose);
      addTearDown(interactionLock.dispose);
      addTearDown(activationRequests.dispose);
      final container = ProviderContainer(
        overrides: [
          composerActivityStateProvider(_scope).overrideWithValue(
            const ComposerActivityState(
              agents: [
                WorkingAgentSignal(
                  pubkey: _agentPubkey,
                  source: AgentWorkingSource.observer,
                  canViewActivity: true,
                  turnId: _turnId,
                ),
              ],
              humanTyping: [],
            ),
          ),
          agentTurnStatesProvider.overrideWithValue([
            _turn(AgentTurnPhase.working),
          ]),
          observerTurnSubscriptionProvider(
            _turnKey,
          ).overrideWithValue(_observerState),
          userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
        ],
      );
      addTearDown(container.dispose);

      await tester.pumpWidget(
        _app(
          container,
          disableAnimations: true,
          composerFocusNode: focusNode,
          composerInteractionLock: interactionLock,
          composerActivationRequests: activationRequests,
        ),
      );
      await tester.pump();
      await tester.tap(
        find.byKey(const ValueKey('composer-agent-activity-control')),
      );
      await tester.pump();
      expect(
        find.byKey(const ValueKey('composer-agent-activity-panel')),
        findsOneWidget,
      );
      expect(interactionLock.value, isTrue);
      expect(focusNode.hasFocus, isFalse);

      activationRequests.value += 1;
      await tester.pump();
      await tester.pump();

      expect(
        find.byKey(const ValueKey('composer-agent-activity-panel')),
        findsNothing,
      );
      expect(interactionLock.value, isFalse);
      expect(focusNode.hasFocus, isTrue);
    },
  );

  testWidgets('an unowned agent is status-only and cannot open activity', (
    tester,
  ) async {
    final container = ProviderContainer(
      overrides: [
        composerActivityStateProvider(_scope).overrideWithValue(
          const ComposerActivityState(
            agents: [
              WorkingAgentSignal(
                pubkey: _secondAgentPubkey,
                source: AgentWorkingSource.typing,
                canViewActivity: false,
              ),
            ],
            humanTyping: [],
          ),
        ),
        agentTurnStatesProvider.overrideWithValue(const []),
        userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
      ],
    );
    addTearDown(container.dispose);

    await tester.pumpWidget(_app(container));
    await tester.pump();

    expect(
      find.byKey(const ValueKey('composer-agent-status-only')),
      findsOneWidget,
    );
    expect(
      find.byKey(const ValueKey('composer-agent-activity-control')),
      findsNothing,
    );
    expect(
      find.byKey(const ValueKey('composer-agent-activity-panel')),
      findsNothing,
    );
  });
}

ProviderContainer _multiAgentContainer() => ProviderContainer(
  overrides: [
    composerActivityStateProvider(_scope).overrideWithValue(
      const ComposerActivityState(
        agents: [
          WorkingAgentSignal(
            pubkey: _agentPubkey,
            source: AgentWorkingSource.observer,
            canViewActivity: true,
            turnId: _turnId,
          ),
          WorkingAgentSignal(
            pubkey: _secondAgentPubkey,
            source: AgentWorkingSource.observer,
            canViewActivity: true,
            turnId: _secondTurnId,
          ),
        ],
        humanTyping: [],
      ),
    ),
    agentTurnStatesProvider.overrideWithValue([
      _turn(AgentTurnPhase.working),
      _turnFor(
        pubkey: _secondAgentPubkey,
        turnId: _secondTurnId,
        phase: AgentTurnPhase.working,
      ),
    ]),
    observerTurnSubscriptionProvider(
      _turnKey,
    ).overrideWithValue(_observerState),
    observerTurnSubscriptionProvider(
      _secondTurnKey,
    ).overrideWithValue(_secondObserverState),
    userCacheProvider.overrideWith(_FakeUserCacheNotifier.new),
  ],
);

Widget _app(
  ProviderContainer container, {
  bool disableAnimations = false,
  double? overlayTopBoundary,
  FocusNode? composerFocusNode,
  TextEditingController? draftController,
  EdgeInsets viewInsets = EdgeInsets.zero,
  double compactWidthFactor = 1,
  Animation<double>? composerWidthAnimation,
  ValueNotifier<bool>? composerInteractionLock,
  ValueNotifier<int>? composerActivationRequests,
  TextScaler textScaler = TextScaler.noScaling,
}) {
  Widget activityIndicator = ComposerAgentActivityIndicator(
    channelId: _channelId,
    overlayTopBoundary: overlayTopBoundary,
    compactWidthFactor: compactWidthFactor,
    composerWidthAnimation: composerWidthAnimation,
    composerFocusNode: composerFocusNode,
    composerInteractionLock: composerInteractionLock,
    composerActivationRequests: composerActivationRequests,
  );
  if (compactWidthFactor < 1) {
    activityIndicator = Padding(
      padding: const EdgeInsets.symmetric(horizontal: Grid.twelve),
      child: Align(
        child: FractionallySizedBox(
          widthFactor: compactWidthFactor,
          child: activityIndicator,
        ),
      ),
    );
  }
  Widget fakeComposer = SizedBox(
    key: const ValueKey('fake-composer'),
    height: 64,
    child: TextField(focusNode: composerFocusNode, controller: draftController),
  );
  if (composerInteractionLock != null) {
    fakeComposer = ValueListenableBuilder<bool>(
      valueListenable: composerInteractionLock,
      builder: (context, interactionLocked, child) =>
          AbsorbPointer(absorbing: interactionLocked, child: child),
      child: fakeComposer,
    );
  }
  return UncontrolledProviderScope(
    container: container,
    child: MaterialApp(
      theme: AppTheme.light(),
      home: Builder(
        builder: (context) => MediaQuery(
          data: MediaQuery.of(context).copyWith(
            disableAnimations: disableAnimations,
            viewInsets: viewInsets,
            textScaler: textScaler,
          ),
          child: Scaffold(
            resizeToAvoidBottomInset: false,
            body: Column(
              children: [
                const Expanded(child: SizedBox()),
                activityIndicator,
                fakeComposer,
              ],
            ),
          ),
        ),
      ),
    ),
  );
}

AgentTurnState _turn(AgentTurnPhase phase) => AgentTurnState(
  agentPubkey: _agentPubkey,
  channelId: _channelId,
  turnId: _turnId,
  startedAt: DateTime.utc(2026, 8, 16, 12),
  lastActivityAt: DateTime.utc(2026, 8, 16, 12, 0, 5),
  livenessTimeout: const Duration(seconds: 30),
  phase: phase,
  terminalAt: phase == AgentTurnPhase.working
      ? null
      : DateTime.utc(2026, 8, 16, 12, 0, 6),
);

AgentTurnState _turnFor({
  required String pubkey,
  required String turnId,
  required AgentTurnPhase phase,
}) => AgentTurnState(
  agentPubkey: pubkey,
  channelId: _channelId,
  turnId: turnId,
  startedAt: DateTime.utc(2026, 8, 16, 12),
  lastActivityAt: DateTime.utc(2026, 8, 16, 12, 0, 5),
  livenessTimeout: const Duration(seconds: 30),
  phase: phase,
);

final _observerState = ObserverState(
  connection: ObserverConnectionState.open,
  transcript: [
    ThoughtItem(
      id: 'thought-1',
      title: 'Thinking',
      text: 'private chain of thought',
      timestamp: '2026-08-16T12:00:01Z',
    ),
    ToolItem(
      id: 'tool-1',
      title: 'Read file',
      toolName: 'read_file',
      status: ToolStatus.executing,
      args: const {'path': '/private/path'},
      result: '',
      isError: false,
      timestamp: '2026-08-16T12:00:02Z',
    ),
  ],
);

final _secondObserverState = ObserverState(
  connection: ObserverConnectionState.open,
  transcript: [
    ToolItem(
      id: 'tool-2',
      title: 'Search messages',
      toolName: 'search',
      status: ToolStatus.executing,
      args: const {},
      result: '',
      isError: false,
      timestamp: '2026-08-16T12:00:02Z',
    ),
  ],
);

final _testComposerActivityProvider =
    NotifierProvider<_TestComposerActivityNotifier, ComposerActivityState>(
      _TestComposerActivityNotifier.new,
    );

class _TestComposerActivityNotifier extends Notifier<ComposerActivityState> {
  @override
  ComposerActivityState build() =>
      const ComposerActivityState(agents: [], humanTyping: []);

  void setState(ComposerActivityState next) => state = next;
}

final _testTurnStatesProvider =
    NotifierProvider<_TestTurnStatesNotifier, List<AgentTurnState>>(
      _TestTurnStatesNotifier.new,
    );

class _TestTurnStatesNotifier extends Notifier<List<AgentTurnState>> {
  @override
  List<AgentTurnState> build() => const [];

  void setStates(List<AgentTurnState> next) => state = next;
}

class _FakeUserCacheNotifier extends UserCacheNotifier {
  @override
  Map<String, UserProfile> build() => const {
    _agentPubkey: UserProfile(
      pubkey: _agentPubkey,
      displayName: 'Pollen',
      ownerPubkey: 'owner',
    ),
    'human-a': UserProfile(pubkey: 'human-a', displayName: 'Alice'),
    _secondAgentPubkey: UserProfile(
      pubkey: _secondAgentPubkey,
      displayName: 'Sprig',
      ownerPubkey: 'owner',
    ),
  };

  @override
  void preload(List<String> pubkeys) {}
}
