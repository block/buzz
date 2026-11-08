import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter/rendering.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

/// Keeps the date anchor visible through a reader's scroll and briefly afterward.
class DateHeaderScrollActivity extends ValueNotifier<bool> {
  DateHeaderScrollActivity() : super(false);

  Timer? _idleTimer;
  bool _readerScrolling = false;

  bool onNotification(ScrollNotification notification) {
    if (notification.depth != 0 || notification.metrics.axis != Axis.vertical) {
      return false;
    }
    if (notification is ScrollStartNotification &&
            notification.dragDetails != null ||
        notification is UserScrollNotification &&
            notification.direction != ScrollDirection.idle) {
      _readerScrolling = true;
    }
    if (_readerScrolling) {
      value = true;
      _idleTimer?.cancel();
      _idleTimer = Timer(const Duration(seconds: 1), () => value = false);
    }
    if (notification is ScrollEndNotification) _readerScrolling = false;
    return false;
  }

  @override
  void dispose() {
    _idleTimer?.cancel();
    super.dispose();
  }
}

DateHeaderScrollActivity useDateHeaderScrollActivity(String timelineId) {
  final activity = useMemoized(DateHeaderScrollActivity.new, [timelineId]);
  useEffect(() => activity.dispose, [activity]);
  return activity;
}
