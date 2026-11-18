import 'dart:async';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../community/community.dart';
import '../security/sensitive_action_authorizer.dart';
import '../theme/theme.dart';
import '../utils/string_utils.dart';
import '../widgets/modal_presentation.dart';
import 'nsec_transfer.dart';

/// Presents explicit plaintext-export consent before device authentication.
Future<void> showNsecExportSheet(
  BuildContext context, {
  required Community community,
  required bool Function() isCurrent,
}) => showBuzzModalBottomSheet<void>(
  context: context,
  title: 'Export private key',
  isScrollControlled: true,
  builder: (_) => NsecExportSheet(community: community, isCurrent: isCurrent),
);

/// Public preview and device-authorized export of the selected community key.
class NsecExportSheet extends HookConsumerWidget {
  const NsecExportSheet({
    super.key,
    required this.community,
    required this.isCurrent,
  });

  final Community community;
  final bool Function() isCurrent;

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final busy = useState(false);
    final error = useState<String?>(null);
    return SafeArea(
      child: SingleChildScrollView(
        padding: const EdgeInsets.all(Grid.gutter),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            Text('Identity for ${community.name}'),
            const SizedBox(height: Grid.xs),
            SelectableText(
              fullNpub(community.pubkey ?? '') ?? 'Identity unavailable',
            ),
            const SizedBox(height: Grid.sm),
            const Text(
              'This file contains your unencrypted private key (nsec). Anyone with it can act as you. Save it somewhere private. Apps you share it with may keep a copy.',
            ),
            const SizedBox(height: Grid.sm),
            if (error.value != null)
              Text(error.value!, style: TextStyle(color: context.colors.error)),
            FilledButton(
              onPressed: busy.value
                  ? null
                  : () async {
                      busy.value = true;
                      error.value = null;
                      final session = ref.read(
                        sensitiveActionAuthorizationSessionProvider,
                      );
                      final box = context.findRenderObject() as RenderBox;
                      final origin = box.localToGlobal(Offset.zero) & box.size;
                      try {
                        final exported = await NsecTransfer.exportCommunity(
                          community: community,
                          authorize: (biometricOnly) =>
                              session.authorize(biometricOnly: biometricOnly),
                          isCurrent: () => context.mounted && isCurrent(),
                          share: (nsec) async {
                            // Native authentication can finish before its app-resume
                            // frame. Do not hand secrets to a background share UI.
                            final resumed = await _waitForResumedFrame();
                            if (!resumed ||
                                !context.mounted ||
                                !isCurrent() ||
                                WidgetsBinding.instance.lifecycleState !=
                                    AppLifecycleState.resumed) {
                              throw StateError('Export interrupted');
                            }
                            return NsecTransfer.shareFile(nsec, origin);
                          },
                        );
                        if (!context.mounted) return;
                        if (exported) {
                          Navigator.of(context).pop();
                        } else {
                          error.value =
                              'Export cancelled. Unlock your device and try again.';
                        }
                      } catch (_) {
                        if (context.mounted) {
                          error.value =
                              'Could not export this identity. Try again.';
                        }
                      } finally {
                        if (context.mounted) busy.value = false;
                      }
                    },
              child: Text(
                busy.value ? 'Authorizing…' : 'Save or share private key',
              ),
            ),
          ],
        ),
      ),
    );
  }
}

Future<bool> _waitForResumedFrame() async {
  final binding = WidgetsBinding.instance;
  if (binding.lifecycleState != AppLifecycleState.resumed) {
    final resumed = Completer<void>();
    final listener = AppLifecycleListener(
      onResume: () {
        if (!resumed.isCompleted) resumed.complete();
      },
    );
    try {
      await resumed.future.timeout(const Duration(seconds: 5));
    } on TimeoutException {
      return false;
    } finally {
      listener.dispose();
    }
  }
  await binding.endOfFrame;
  return true;
}
