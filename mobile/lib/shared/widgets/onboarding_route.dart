import 'package:flutter/material.dart';

/// A standard onboarding route that skips motion when the OS requests it.
Route<T> onboardingRoute<T>(
  BuildContext context, {
  required WidgetBuilder builder,
  RouteSettings? settings,
}) {
  if (MediaQuery.disableAnimationsOf(context)) {
    return PageRouteBuilder<T>(
      settings: settings,
      transitionDuration: Duration.zero,
      reverseTransitionDuration: Duration.zero,
      pageBuilder: (context, _, _) => Semantics(
        scopesRoute: true,
        explicitChildNodes: true,
        child: builder(context),
      ),
    );
  }
  return MaterialPageRoute<T>(settings: settings, builder: builder);
}

/// Declarative counterpart of [onboardingRoute] for nested onboarding steps.
class OnboardingPage<T> extends MaterialPage<T> {
  const OnboardingPage({super.key, required super.child});

  @override
  Route<T> createRoute(BuildContext context) {
    if (!MediaQuery.disableAnimationsOf(context)) {
      return super.createRoute(context);
    }
    return _InstantOnboardingRoute<T>(this);
  }
}

class _InstantOnboardingRoute<T> extends PageRouteBuilder<T> {
  _InstantOnboardingRoute(OnboardingPage<T> page)
    : super(
        settings: page,
        transitionDuration: Duration.zero,
        reverseTransitionDuration: Duration.zero,
        pageBuilder: (_, _, _) => const SizedBox.shrink(),
      );

  @override
  Widget buildPage(
    BuildContext context,
    Animation<double> animation,
    Animation<double> secondaryAnimation,
  ) => Semantics(
    scopesRoute: true,
    explicitChildNodes: true,
    child: (settings as OnboardingPage<T>).child,
  );
}
