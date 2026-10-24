import 'package:flutter/cupertino.dart';
import 'package:flutter/foundation.dart';
import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../hosted/account_identity_store.dart';
import '../theme/theme.dart';
import '../utils/string_utils.dart';
import '../widgets/modal_presentation.dart';
import 'nsec_transfer.dart';

/// Lets the user inspect the public identity before explicitly importing it.
/// [onImport] is the secure persistence boundary, not an observable UI store.
Future<bool> showNsecImportSheet(
  BuildContext context, {
  required Future<void> Function(String nsec) onImport,
}) async =>
    await showBuzzModalBottomSheet<bool>(
      context: context,
      title: 'Import identity',
      isScrollControlled: true,
      builder: (_) => NsecImportSheet(onImport: onImport),
    ) ??
    false;

/// Transient secret input; only public preview and safe errors are displayed.
class NsecImportSheet extends HookWidget {
  const NsecImportSheet({
    super.key,
    required this.onImport,
    this.pickFile,
    this.autofocus = true,
  });

  final Future<void> Function(String nsec) onImport;
  final Future<String?> Function()? pickFile;

  /// Allows a containing sheet to finish expanding before opening the keyboard.
  final bool autofocus;

  @override
  Widget build(BuildContext context) {
    final input = useTextEditingController();
    final focus = useFocusNode();
    final autofocusRef = useRef(autofocus)..value = autofocus;
    useEffect(() {
      if (autofocus && defaultTargetPlatform == TargetPlatform.iOS) {
        WidgetsBinding.instance.addPostFrameCallback((_) {
          if (context.mounted && autofocusRef.value) focus.requestFocus();
        });
      }
      return null;
    }, [autofocus]);
    final publicKey = useState<String?>(null);
    final error = useState<String?>(null);
    final busy = useState(false);

    Future<void> chooseFile() async {
      busy.value = true;
      error.value = null;
      try {
        final text = await (pickFile ?? NsecTransfer.pickFile)();
        if (!context.mounted || text == null) return;
        final preview = NsecTransfer.preview(text);
        input.text = text.trim();
        publicKey.value = preview;
      } catch (_) {
        if (context.mounted) {
          error.value = 'Choose a text file containing one valid nsec.';
        }
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    Future<void> submit() async {
      error.value = null;
      try {
        final preview = NsecTransfer.preview(input.text);
        if (publicKey.value != preview) {
          publicKey.value = preview;
          return;
        }
        busy.value = true;
        await onImport(input.text.trim());
        if (!context.mounted) return;
        input.clear();
        Navigator.of(context).pop(true);
      } catch (failure) {
        if (!context.mounted) return;
        error.value = switch (failure) {
          AccountIdentityFailure.occupied || AccountIdentityFailure.mismatch =>
            'This device already has a different identity. It has not been replaced.',
          AccountIdentityFailure.invalidKey =>
            'Enter a valid nsec private key.',
          _ =>
            'Could not finish importing this identity. Try again with the same key.',
        };
      } finally {
        if (context.mounted) busy.value = false;
      }
    }

    return SafeArea(
      child: SingleChildScrollView(
        padding: EdgeInsets.fromLTRB(
          Grid.gutter,
          Grid.xs,
          Grid.gutter,
          Grid.gutter + MediaQuery.viewInsetsOf(context).bottom,
        ),
        child: Column(
          mainAxisSize: MainAxisSize.min,
          crossAxisAlignment: CrossAxisAlignment.stretch,
          children: [
            if (defaultTargetPlatform == TargetPlatform.iOS)
              CupertinoTextField(
                controller: input,
                placeholder: 'Private key (nsec)',
                padding: const EdgeInsets.all(Grid.gutter),
                decoration: BoxDecoration(
                  color: CupertinoColors.secondarySystemGroupedBackground
                      .resolveFrom(context),
                  borderRadius: BorderRadius.circular(Radii.container),
                ),
                focusNode: focus,
                maxLength: 1024,
                enabled: !busy.value,
                obscureText: true,
                autocorrect: false,
                enableSuggestions: false,
                enableIMEPersonalizedLearning: false,
                onChanged: (_) {
                  publicKey.value = null;
                  error.value = null;
                },
              )
            else
              TextField(
                controller: input,
                maxLength: 1024,
                enabled: !busy.value,
                obscureText: true,
                autocorrect: false,
                enableSuggestions: false,
                enableIMEPersonalizedLearning: false,
                decoration: const InputDecoration(
                  labelText: 'Private key (nsec)',
                ),
                onChanged: (_) {
                  publicKey.value = null;
                  error.value = null;
                },
              ),
            const SizedBox(height: Grid.xs),
            OutlinedButton(
              style: OutlinedButton.styleFrom(
                minimumSize: const Size.fromHeight(52),
                shape: const StadiumBorder(),
              ),
              onPressed: busy.value ? null : chooseFile,
              child: const Text('Choose nsec file'),
            ),
            if (publicKey.value != null) ...[
              const Text(
                'Confirm this is the public identity you want to use:',
              ),
              const SizedBox(height: Grid.xs),
              SelectableText(fullNpub(publicKey.value!)!),
              const SizedBox(height: Grid.sm),
            ],
            if (error.value != null)
              Text(error.value!, style: TextStyle(color: context.colors.error)),
            const SizedBox(height: Grid.xs),
            FilledButton(
              style: FilledButton.styleFrom(
                minimumSize: const Size.fromHeight(52),
                shape: const StadiumBorder(),
              ),
              onPressed: busy.value ? null : submit,
              child: Text(
                busy.value
                    ? 'Importing…'
                    : publicKey.value == null
                    ? 'Preview identity'
                    : 'Import this identity',
              ),
            ),
          ],
        ),
      ),
    );
  }
}
