import 'package:flutter/gestures.dart';
import 'community_link_focus.dart';
import '../../shared/widgets/ios_community_link_input.dart';
import 'package:flutter/material.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/deeplink/community_link_input.dart';
import '../../shared/theme/theme.dart';
import '../../shared/widgets/onboarding_step_page.dart';

part 'add_community_page/add_community_form.dart';
part 'add_community_page/add_community_connection.dart';
part 'add_community_page/pairing_reveal_surface.dart';

/// Fullscreen Add Community presentation; connection operations belong to callers.
class AddCommunityPage extends StatelessWidget {
  const AddCommunityPage({
    super.key,
    required this.onBack,
    required this.onCommunityLink,
    required this.onPairing,
    required this.onScan,
    required this.onOpenCommunity,
    required this.onSubmitPairing,
    this.enteringCommunityLink = false,
    this.existingUser = false,
    this.scannerProgress = 0,
    this.busy = false,
    this.canSubmit = true,
    this.errorMessage,
    this.accountStatus,
    this.onInputChanged,
  });

  final VoidCallback onBack;
  final VoidCallback? onCommunityLink;
  final VoidCallback? onPairing;
  final VoidCallback? onScan;
  final ValueChanged<Uri> onOpenCommunity;
  final ValueChanged<String> onSubmitPairing;
  final bool enteringCommunityLink;
  final bool existingUser;
  final double scannerProgress;
  final bool busy;
  final bool canSubmit;
  final String? errorMessage;
  final String? accountStatus;
  final VoidCallback? onInputChanged;

  @override
  Widget build(BuildContext context) {
    final step = enteringCommunityLink
        ? 'link'
        : existingUser
        ? 'pair'
        : 'options';
    final page = step == 'options' || step == 'link'
        ? _AddCommunityConnection(
            key: const ValueKey('add-community-connection'),
            enteringLink: enteringCommunityLink,
            scannerProgress: scannerProgress,
            accountStatus: accountStatus,
            onBack: onBack,
            onCommunityLink: onCommunityLink,
            onPairing: onPairing,
            onScan: onScan,
            onOpenCommunity: onOpenCommunity,
            busy: busy,
            canSubmit: canSubmit,
            errorMessage: errorMessage,
            onInputChanged: onInputChanged,
          )
        : _AddCommunityForm(
            key: ValueKey(step),
            onBack: onBack,
            onSubmitPairing: onSubmitPairing,
            busy: busy,
            canSubmit: canSubmit,
            errorMessage: errorMessage,
            onInputChanged: onInputChanged,
          );
    return AnimatedSwitcher(
      duration: MediaQuery.disableAnimationsOf(context)
          ? Duration.zero
          : const Duration(milliseconds: 250),
      layoutBuilder: (current, previous) => Stack(
        fit: StackFit.expand,
        children: [
          for (final child in previous)
            ExcludeFocus(
              child: ExcludeSemantics(child: IgnorePointer(child: child)),
            ),
          ?current,
        ],
      ),
      transitionBuilder: (child, animation) => SlideTransition(
        position: Tween(begin: const Offset(1, 0), end: Offset.zero).animate(
          CurvedAnimation(parent: animation, curve: Curves.easeOutCubic),
        ),
        child: child,
      ),
      child: page,
    );
  }
}
