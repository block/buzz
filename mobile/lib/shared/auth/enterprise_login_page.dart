import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import '../theme/grid.dart';
import 'auth_provider.dart';
import 'enterprise_identity.dart';

/// Corporate builds never offer local-key pairing as a fallback.
class EnterpriseLoginPage extends HookConsumerWidget {
  const EnterpriseLoginPage({super.key});
  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final busy = useState(false);
    final error = useState<String?>(null);
    return Scaffold(
      body: SafeArea(
        child: Center(
          child: Padding(
            padding: const EdgeInsets.all(Grid.sm),
            child: Column(
              mainAxisSize: MainAxisSize.min,
              children: [
                const Text('Sign in to Buzz for work'),
                const SizedBox(height: Grid.xs),
                const Text(
                  'Your organization manages your identity and community.',
                ),
                if (error.value != null)
                  Semantics(liveRegion: true, child: Text(error.value!)),
                const SizedBox(height: Grid.sm),
                FilledButton(
                  onPressed: busy.value
                      ? null
                      : () async {
                          busy.value = true;
                          error.value = null;
                          try {
                            await EnterpriseIdentity.instance.login();
                            ref.invalidate(authProvider);
                          } catch (_) {
                            if (context.mounted) {
                              error.value =
                                  'Sign-in did not complete. Try again.';
                            }
                          } finally {
                            if (context.mounted) busy.value = false;
                          }
                        },
                  child: Text(
                    busy.value
                        ? 'Waiting for corporate sign-in…'
                        : 'Sign in with corporate account',
                  ),
                ),
              ],
            ),
          ),
        ),
      ),
    );
  }
}
