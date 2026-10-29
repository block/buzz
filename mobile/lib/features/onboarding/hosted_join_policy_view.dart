import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import '../../shared/hosted/hosted_join_policy.dart';
import '../../shared/theme/grid.dart';
import '../../shared/widgets/onboarding_step_page.dart';

/// Consent is explicit and scoped to the policy version shown on this screen.
class HostedJoinPolicyView extends HookWidget {
  const HostedJoinPolicyView({
    super.key,
    required this.policy,
    required this.onAccept,
    this.fullScreen = false,
    this.onBack,
  });
  final HostedJoinPolicy policy;
  final VoidCallback onAccept;
  final bool fullScreen;
  final VoidCallback? onBack;

  @override
  Widget build(BuildContext context) {
    final ageConfirmed = useState(false);
    final children = <Widget>[
      if (policy.terms != null)
        ExpansionTile(
          title: const Text('Terms of service'),
          children: [
            Padding(
              padding: const EdgeInsets.all(Grid.sm),
              child: SelectableText(policy.terms!),
            ),
          ],
        ),
      if (policy.privacy != null)
        ExpansionTile(
          title: const Text('Privacy policy'),
          children: [
            Padding(
              padding: const EdgeInsets.all(Grid.sm),
              child: SelectableText(policy.privacy!),
            ),
          ],
        ),
      if (policy.ageRequired)
        CheckboxListTile(
          value: ageConfirmed.value,
          onChanged: (value) => ageConfirmed.value = value ?? false,
          title: const Text('I am 18 or older'),
        ),
      const SizedBox(height: Grid.sm),
      FilledButton(
        onPressed: !policy.ageRequired || ageConfirmed.value ? onAccept : null,
        child: const Text('Accept and join'),
      ),
    ];
    if (fullScreen) {
      return OnboardingStepPage(
        title: 'Join your community',
        subtitle:
            'Review this community’s terms and privacy policy before joining.',
        onBack: onBack ?? () {},
        content: Column(
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: children.sublist(0, children.length - 1),
        ),
        actions: [children.last],
      );
    }
    return Column(
      crossAxisAlignment: CrossAxisAlignment.stretch,
      children: children,
    );
  }
}
