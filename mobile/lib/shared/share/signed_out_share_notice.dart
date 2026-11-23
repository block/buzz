import 'dart:async';

import 'package:flutter/widgets.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../deeplink/deep_link.dart';
import '../deeplink/pending_deep_link_provider.dart';
import '../widgets/buzz_toast.dart';
import 'pending_share.dart';

/// Turns away a share that arrives while signed out.
///
/// Shares open a conversation, so they need an account. They are not held
/// for after sign-in: the conversation they name belongs to a sign-in that no
/// longer exists on this device.
class SignedOutShareNotice extends HookConsumerWidget {
  const SignedOutShareNotice({required this.child, super.key});

  final Widget child;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final link = ref.watch(pendingDeepLinkProvider);
    useEffect(() {
      if (link is! ShareDeepLink) return null;
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (!context.mounted || ref.read(pendingDeepLinkProvider) != link) {
          return;
        }
        ref.read(pendingDeepLinkProvider.notifier).consume();
        unawaited(deletePendingShare(link.shareId));
        showBuzzToast(context, 'Sign in to share', kind: ToastKind.error);
      });
      return null;
    }, [link]);
    return child;
  }
}
