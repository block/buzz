import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';

import '../../shared/theme/theme.dart';
import '../../shared/widgets/frosted_app_bar.dart';
import '../../shared/widgets/frosted_scaffold.dart';
import 'agent_avatar_picker.dart';

/// Edits an avatar locally; Cancel leaves the creation draft unchanged.
class AgentAvatarEditorPage extends HookWidget {
  const AgentAvatarEditorPage({super.key, required this.initialSelection});
  final AgentAvatarSelection initialSelection;

  @override
  Widget build(BuildContext context) {
    final selection = useState(initialSelection);
    final busy = useState(false);
    void done() => Navigator.of(context).pop(selection.value);
    void cancel() => Navigator.of(context).pop();
    return PopScope(
      canPop: !busy.value,
      child: FrostedScaffold(
        appBar: FrostedAppBar(
          nativeTitle: 'Choose avatar',
          title: const Text('Choose avatar'),
          automaticallyImplyLeading: false,
          nativeLeading: IosNavigationAction(
            label: 'Cancel',
            symbol: 'xmark',
            onPressed: busy.value ? null : cancel,
          ),
          nativeActions: [
            IosNavigationAction(
              label: 'Done',
              symbol: 'checkmark',
              onPressed: busy.value ? null : done,
            ),
          ],
          leading: IconButton(
            tooltip: 'Cancel',
            onPressed: busy.value ? null : cancel,
            icon: const Icon(Icons.close),
          ),
          actions: [
            IconButton(
              tooltip: 'Done',
              onPressed: busy.value ? null : done,
              icon: const Icon(Icons.check),
            ),
          ],
        ),
        body: SafeArea(
          top: false,
          child: SingleChildScrollView(
            padding: EdgeInsets.fromLTRB(
              Grid.gutter,
              frostedAppBarHeight(context) + Grid.md,
              Grid.gutter,
              Grid.md,
            ),
            child: AgentAvatarPicker(
              selection: selection.value,
              onChanged: (value) => selection.value = value,
              disabled: false,
              onBusyChanged: (value) => busy.value = value,
            ),
          ),
        ),
      ),
    );
  }
}
