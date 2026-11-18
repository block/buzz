import 'package:flutter/material.dart';

/// The app's bottom route. It refuses to be popped: removing it would leave an
/// empty navigator, a blank screen with nothing to return to.
///
/// A pop that reaches it is a caller bug (for example, popping "whatever is
/// on top" after an await, when the route that meant to close is already
/// gone). The pop is reported as an error, which fails tests, and ignored.
class RootPageRoute<T> extends MaterialPageRoute<T> {
  RootPageRoute({required super.builder, super.settings});

  // Not conditioned on [isFirst]: while a pop is in progress the route no
  // longer counts as present, so [isFirst] already reads false here.
  // Calling super would complete and remove this route, the pop it refuses.
  @override
  // ignore: must_call_super
  bool didPop(T? result) {
    // A drawer or persistent sheet on the root route closes through this pop.
    if (willHandlePopInternally) return super.didPop(result);
    FlutterError.reportError(
      FlutterErrorDetails(
        exception: StateError('Ignored a pop of the root route.'),
        stack: StackTrace.current,
        library: 'buzz navigation',
        context: ErrorDescription('while popping the root route'),
      ),
    );
    return false;
  }
}
