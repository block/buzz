import 'dart:async';

import 'package:flutter/material.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../../shared/theme/theme.dart';

/// Only the newest history group can consume a pending live disclosure.
typedef ActivityScope = ({String channel, String author, String? root});
final pendingActivityExpansionProvider = NotifierProvider.autoDispose
    .family<PendingActivityExpansion, bool, ActivityScope>(
      PendingActivityExpansion.new,
    );

class PendingActivityExpansion extends Notifier<bool> {
  PendingActivityExpansion(this.scope);
  final ActivityScope scope;
  bool _expanded = false;
  Timer? _expiry;
  void Function()? _release;
  @override
  bool build() {
    ref.onDispose(() => _expiry?.cancel());
    ref.onResume(() => _expiry?.cancel());
    ref.onCancel(() {
      if (!_expanded) return;
      _expiry?.cancel();
      _expiry = Timer(const Duration(seconds: 30), () {
        _expanded = false;
        state = false;
        _release?.call();
        _release = null;
      });
    });
    return false;
  }

  void setExpanded(bool value) {
    _expanded = value;
    state = value;
    _expiry?.cancel();
    _release?.call();
    _release = null;
    if (value) {
      // Typing and signed-message subscriptions can deliver on separate frames.
      final link = ref.keepAlive();
      _release = link.close;
    }
  }
}

class ActivityMessageWidth extends StatelessWidget {
  const ActivityMessageWidth({
    super.key,
    required this.child,
    required this.showAvatar,
  });
  final Widget child;
  final bool showAvatar;

  @override
  Widget build(BuildContext context) => Align(
    alignment: Alignment.topLeft,
    child: ConstrainedBox(
      key: const ValueKey('activity-message-width'),
      constraints: BoxConstraints(
        maxWidth:
            MediaQuery.sizeOf(context).width * messageTextMaxWidthFraction +
            (showAvatar ? 0 : messageAvatarSize + messageAvatarContentGap),
      ),
      child: child,
    ),
  );
}
