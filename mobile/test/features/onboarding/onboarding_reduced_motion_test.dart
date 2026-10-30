import 'package:buzz/features/onboarding/hosted_connect_page.dart';
import 'package:buzz/features/pairing/add_community_page.dart';
import 'package:flutter/material.dart';
import 'package:flutter_secure_storage/flutter_secure_storage.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../helpers/hosted_communities_fixture.dart';

void main() {
  testWidgets('reduced motion swaps Add and hosted steps instantly', (
    tester,
  ) async {
    final pair = ValueNotifier(false);
    addTearDown(pair.dispose);
    Widget app(Widget home, {List<NavigatorObserver> observers = const []}) =>
        ProviderScope(
          child: MaterialApp(
            navigatorObservers: observers,
            builder: (context, child) => MediaQuery(
              data: MediaQuery.of(context).copyWith(disableAnimations: true),
              child: child!,
            ),
            home: home,
          ),
        );
    await tester.pumpWidget(
      app(
        ValueListenableBuilder<bool>(
          valueListenable: pair,
          builder: (_, pairing, _) => AddCommunityPage(
            existingUser: pairing,
            onBack: () => pair.value = false,
            onCommunityLink: () {},
            onPairing: () => pair.value = true,
            onScan: () {},
            onOpenCommunity: (_) {},
            onSubmitPairing: (_) {},
          ),
        ),
      ),
    );
    await tester.pumpAndSettle();
    pair.value = true;
    await tester.pump();
    await tester.pump();
    final pairRoute = ModalRoute.of(
      tester.element(find.byType(TextField).first),
    )!;
    expect(pairRoute.animation!.isCompleted, isTrue);
    expect(pairRoute.transitionDuration, Duration.zero);
    expect(find.text('Add a community'), findsNothing);
    pair.value = false;
    await tester.pump();
    await tester.pump();
    expect(find.text('Add a community'), findsOneWidget);
    expect(find.byKey(const Key('community-pairing-input')), findsNothing);
    await tester.pumpWidget(const SizedBox());
    await tester.pumpAndSettle();

    FlutterSecureStorage.setMockInitialValues({});
    final f = HostedCommunitiesFixture();
    await f.initialize(widget: true);
    addTearDown(f.dispose);
    f.rows = [f.row('one'), f.row('two')];
    final observer = _Routes();
    var closes = 0;
    await tester.pumpWidget(
      app(
        HostedConnectPage(
          flow: f.flow,
          onClose: () => closes++,
          profileBuilder: (_) => const Text('Profile content'),
        ),
        observers: [observer],
      ),
    );
    await tester.pump();
    await tester.pump();
    final outer =
        observer.routes.singleWhere((r) => r.settings.name == 'hosted-setup')
            as TransitionRoute;
    expect(outer.transitionDuration, Duration.zero);
    expect(outer.animation!.isCompleted, isTrue);
    await tester.binding.handlePopRoute();
    await tester.pump();
    expect(closes, 1);
    await f.flow.signInAccount();
    await tester.pump();
    await tester.pump();
    final title = find.text('Select a community');
    expect(title, findsOneWidget);
    final inner = ModalRoute.of(tester.element(title))!;
    expect(inner.transitionDuration, Duration.zero);
    expect(inner.animation!.isCompleted, isTrue);
    // Profile uses the same page key; updated child content must still rebuild.
    final profile = f.flow.requestProfile((_) async {});
    await tester.pump();
    await tester.pump();
    expect(find.text('Profile content'), findsOneWidget);
    f.flow.finishProfile();
    await profile;
    await tester.pumpWidget(const SizedBox());
    await tester.pumpAndSettle();
  });
}

class _Routes extends NavigatorObserver {
  final routes = <Route<dynamic>>[];
  @override
  void didPush(Route<dynamic> route, Route<dynamic>? previousRoute) =>
      routes.add(route);
}
