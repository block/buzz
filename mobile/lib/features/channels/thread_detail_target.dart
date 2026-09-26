import 'package:flutter/widgets.dart';

import 'timeline_message.dart';

class ThreadDetailTarget {
  const ThreadDetailTarget({
    required this.threadHead,
    required this.allMessages,
    required this.channelId,
    required this.currentPubkey,
    required this.isMember,
    required this.isArchived,
    this.initialMessageId,
  });

  final TimelineMessage threadHead;
  final List<TimelineMessage> allMessages;
  final String channelId;
  final String? currentPubkey;
  final bool isMember;
  final bool isArchived;
  final String? initialMessageId;
}

class ThreadDetailPaneScope extends InheritedWidget {
  const ThreadDetailPaneScope({
    required this.onOpenThread,
    required super.child,
    super.key,
  });

  final ValueChanged<ThreadDetailTarget> onOpenThread;

  static ThreadDetailPaneScope? maybeOf(BuildContext context) =>
      context.getInheritedWidgetOfExactType<ThreadDetailPaneScope>();

  @override
  bool updateShouldNotify(ThreadDetailPaneScope _) => false;
}
