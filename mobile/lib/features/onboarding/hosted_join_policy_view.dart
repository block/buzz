import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import '../../shared/hosted/hosted_join_policy.dart';
import '../../shared/theme/grid.dart';

/// Consent is explicit and scoped to the policy version shown on this screen.
class HostedJoinPolicyView extends HookWidget {
  const HostedJoinPolicyView({
    super.key,
    required this.policy,
    required this.onAccept,
  });
  final HostedJoinPolicy policy;
  final VoidCallback onAccept;

  @override
  Widget build(BuildContext context) {
    final ageConfirmed = useState(false);
    return Material(
      type: MaterialType.transparency,
      child: Column(
        crossAxisAlignment: CrossAxisAlignment.stretch,
        children: [
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
            onPressed: !policy.ageRequired || ageConfirmed.value
                ? onAccept
                : null,
            child: const Text('Accept and join'),
          ),
        ],
      ),
    );
  }
}
