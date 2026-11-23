import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';

import '../shared/theme/grid.dart';
import 'share_bridge.dart';
import 'share_conversations.dart';

part 'share_sheet/destination_list.dart';
part 'share_sheet/send.dart';

const _bridge = ShareBridge();

/// The share extension's screen: a note, a destination and Send. Sending
/// stays in the extension; only a new Solo chat opens the app.
class ShareSheet extends HookConsumerWidget {
  const ShareSheet({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final payload = useMemoized(_bridge.load);
    final loaded = useFuture(payload);
    final data = loaded.data;
    if (data == null) {
      return Scaffold(
        body: Center(
          child: loaded.hasError
              ? Text('${loaded.error}')
              : const CircularProgressIndicator(),
        ),
      );
    }
    if (data.community == null) {
      return _Message(
        text: data.signedOut ?? 'Sign in to Buzz to share from here.',
      );
    }
    return _ShareForm(payload: data);
  }
}

class _ShareForm extends HookWidget {
  const _ShareForm({required this.payload});

  final SharePayload payload;

  @override
  Widget build(BuildContext context) {
    final note = useTextEditingController(text: payload.note);
    final search = useTextEditingController();
    useListenable(search);
    final destinations = useMemoized(
      () => loadShareDestinations(_bridge, payload),
    );
    final loaded = useFuture(destinations);
    final selected = useState<String?>(payload.suggested);
    final send = useState<_SendState>(const _SendIdle());
    final upload = useRef<Map<String, Object?>?>(null);
    final createdAt = useRef<int?>(null);
    final all = loaded.data;
    final conversations = [?all?.bestie, ...?all?.recent];
    final target = conversations
        .where((c) => c.id == selected.value)
        .firstOrNull;
    final busy = send.value is _Sending;

    Future<void> submit() async {
      if (target == null) return;
      // A changed destination or note is a new message, not a retry.
      createdAt.value ??= DateTime.now().millisecondsSinceEpoch ~/ 1000;
      send.value = const _Sending('Sending…');
      try {
        await sendShare(
          payload: payload,
          note: note.text,
          target: target,
          createdAt: createdAt.value!,
          upload: upload,
          onUploading: () => send.value = const _Sending('Uploading photo…'),
        );
        await _bridge.close();
      } on Object catch (error) {
        send.value = _SendFailed(_describe(error));
      }
    }

    return Scaffold(
      appBar: AppBar(
        leading: TextButton(
          onPressed: busy ? null : _bridge.cancel,
          child: const Text('Cancel'),
        ),
        leadingWidth: 88,
        title: Text(payload.community!.name),
        actions: [
          TextButton(
            onPressed: busy || target == null ? null : submit,
            child: const Text('Send'),
          ),
        ],
      ),
      body: Column(
        children: [
          if (busy) const LinearProgressIndicator(),
          Padding(
            padding: const EdgeInsets.all(Grid.xs),
            child: Row(
              crossAxisAlignment: CrossAxisAlignment.start,
              children: [
                if (payload.imagePath case final path?)
                  Padding(
                    padding: const EdgeInsets.only(right: Grid.twelve),
                    child: ClipRRect(
                      borderRadius: BorderRadius.circular(8),
                      child: Image.file(
                        File(path),
                        width: 72,
                        height: 72,
                        fit: BoxFit.cover,
                        cacheWidth: 216,
                        semanticLabel: 'Shared photo',
                      ),
                    ),
                  ),
                Expanded(
                  child: TextField(
                    controller: note,
                    enabled: !busy,
                    minLines: 2,
                    maxLines: 5,
                    decoration: InputDecoration(
                      hintText: payload.imagePath == null
                          ? 'Add a note'
                          : 'Add a caption',
                      border: const OutlineInputBorder(),
                    ),
                  ),
                ),
              ],
            ),
          ),
          if (payload.problem case final problem?)
            _Notice(text: problem, isError: true),
          if (send.value case _SendFailed(:final message))
            _Notice(
              text: message,
              isError: true,
              action: TextButton(onPressed: submit, child: const Text('Retry')),
            ),
          if (send.value case _Sending(:final label)) _Notice(text: label),
          Expanded(
            child: _DestinationList(
              loaded: loaded,
              search: search,
              selected: selected.value,
              enabled: !busy,
              onSelect: (id) {
                if (id != selected.value) createdAt.value = null;
                selected.value = id;
              },
              onNewSoloChat: () => _bridge.openApp(note.text),
            ),
          ),
        ],
      ),
    );
  }
}

String _describe(Object error) {
  final text = '$error';
  final match = RegExp(
    r'PlatformException\([^,]*, (.*), null',
  ).firstMatch(text);
  return match?.group(1) ?? text;
}

class _Notice extends StatelessWidget {
  const _Notice({required this.text, this.isError = false, this.action});

  final String text;
  final bool isError;
  final Widget? action;

  @override
  Widget build(BuildContext context) {
    final colors = Theme.of(context).colorScheme;
    return Padding(
      padding: const EdgeInsets.symmetric(horizontal: Grid.xs),
      child: Row(
        children: [
          Expanded(
            child: Text(
              text,
              style: TextStyle(color: isError ? colors.error : null),
            ),
          ),
          ?action,
        ],
      ),
    );
  }
}

class _Message extends StatelessWidget {
  const _Message({required this.text});

  final String text;

  @override
  Widget build(BuildContext context) {
    return Scaffold(
      appBar: AppBar(
        leading: TextButton(
          onPressed: _bridge.cancel,
          child: const Text('Close'),
        ),
        leadingWidth: 88,
      ),
      body: Center(
        child: Padding(
          padding: const EdgeInsets.all(Grid.sm),
          child: Text(text, textAlign: TextAlign.center),
        ),
      ),
    );
  }
}
