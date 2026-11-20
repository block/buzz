import 'package:flutter/foundation.dart';

import 'diagnostic_reporter.dart';

/// Counting a networking event and leaving its breadcrumb are one step for
/// app code: every counted event is also on the trail, except where a caller
/// opts out because the trail would drown in routine successes.
extension NetworkEventReporting on DiagnosticReporter {
  /// Counts [event] and, unless [breadcrumb] is false, adds it to the trail.
  ///
  /// The reporter's own calls never throw. This guards the request path
  /// again anyway, because the spec lets nothing in reporting reach it.
  void noteNetworkEvent(NetworkEvent event, {bool breadcrumb = true}) {
    try {
      countNetworkEvent(event);
      if (breadcrumb) leaveBreadcrumb(DiagnosticBreadcrumb(event));
    } on Object catch (error) {
      debugPrint('[Diagnostics] count dropped: ${error.runtimeType}');
    }
  }
}
