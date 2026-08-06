part of '../settings_page.dart';

class _MobileSecuritySection extends ConsumerWidget {
  const _MobileSecuritySection();

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final auth = ref.watch(authProvider).value;
    final community = auth?.community;
    if (community == null) return const SizedBox.shrink();

    final enabled =
        community.sensitiveActionPolicy == SensitiveActionPolicy.enabled;
    final capability = ref.watch(sensitiveActionAuthSupportedProvider);
    final authenticationName = sensitiveActionAuthenticationName(
      Theme.of(context).platform,
    );

    return AppListCard(
      label: 'Mobile security',
      children: [
        SwitchListTile(
          key: const Key('sensitive-action-confirmation-toggle'),
          secondary: const Icon(LucideIcons.shieldCheck),
          title: const Text('Confirm sensitive identity actions'),
          subtitle: Text(
            enabled
                ? 'Device authentication is required before sending your identity to a desktop.'
                : 'Routine Buzz use never prompts. Enable protection for identity transfers.',
          ),
          value: enabled,
          onChanged: (value) => _changePolicy(context, ref, value),
        ),
        AppListRow(
          icon: LucideIcons.fingerprint,
          title: 'Device authentication',
          subtitle: capability.when(
            data: (supported) => supported
                ? '${authenticationName[0].toUpperCase()}${authenticationName.substring(1)} or device passcode available'
                : 'Unavailable or not configured',
            loading: () => 'Checking…',
            error: (_, _) => 'Unavailable',
          ),
        ),
      ],
    );
  }

  Future<void> _changePolicy(
    BuildContext context,
    WidgetRef ref,
    bool enabled,
  ) async {
    final currentPolicy = ref
        .read(authProvider)
        .value
        ?.community
        ?.sensitiveActionPolicy;
    if (enabled || currentPolicy == SensitiveActionPolicy.enabled) {
      final result = await ref
          .read(sensitiveActionAuthorizerProvider)
          .authorizeIdentityAction();
      if (!context.mounted) return;
      if (result != DeviceAuthResult.success) {
        ScaffoldMessenger.of(context).showSnackBar(
          const SnackBar(
            content: Text('Device authentication did not complete.'),
          ),
        );
        return;
      }
    }
    await ref
        .read(authProvider.notifier)
        .updateSensitiveActionPolicy(
          enabled
              ? SensitiveActionPolicy.enabled
              : SensitiveActionPolicy.disabledByUser,
        );
  }
}
