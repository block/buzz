import 'dart:io';

import 'package:flutter/material.dart';
import 'package:flutter_hooks/flutter_hooks.dart';
import 'package:hooks_riverpod/hooks_riverpod.dart';
import 'package:uuid/uuid.dart';

import '../features/channels/channel_management_provider.dart'
    show buildCreateChannelTags;
import '../shared/me/me_chat_title.dart';
import '../shared/relay/animated_image_sanitizer.dart';
import '../shared/relay/media_upload.dart'
    show BlobDescriptor, safeAttachmentFilename;

import '../shared/theme/buzz_icons.dart';
import '../shared/theme/grid.dart';
import 'share_bridge.dart';
import 'share_conversations.dart';

part 'share_sheet/attachment_previews.dart';
part 'share_sheet/destination_list.dart';
part 'share_sheet/send.dart';

const _bridge = ShareBridge();

/// The share extension's screen: a note, a destination and Send. Sending
/// stays in the extension, including starting a new Solo chat.
class ShareSheet extends HookConsumerWidget {
  const ShareSheet({super.key});

  @override
  Widget build(BuildContext context, WidgetRef ref) {
    final payload = useMemoized(_bridge.load);
    final loaded = useFuture(payload);
    final switched = useState<SharePayload?>(null);
    final switching = useState(false);
    final data = switched.value ?? loaded.data;
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
    Future<void> switchTo(String id) async {
      if (id == data.community!.id || switching.value) return;
      switching.value = true;
      try {
        switched.value = await _bridge.switchCommunity(data, id);
      } on Object catch (error) {
        if (context.mounted) {
          ScaffoldMessenger.of(
            context,
          ).showSnackBar(SnackBar(content: Text(_describe(error))));
        }
      } finally {
        switching.value = false;
      }
    }

    // A new community starts a new form: its own destinations, and uploads
    // to its own relay.
    return _ShareForm(
      key: ValueKey(data.community!.id),
      payload: data,
      onSwitchCommunity: switching.value ? null : switchTo,
    );
  }
}

class _ShareForm extends HookWidget {
  const _ShareForm({
    super.key,
    required this.payload,
    required this.onSwitchCommunity,
  });

  final SharePayload payload;
  final ValueChanged<String>? onSwitchCommunity;

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
    final uploads = useMemoized(() => <int, BlobDescriptor>{});
    final createdAt = useRef<int?>(null);
    final solo = useRef<SoloChatProgress?>(null);
    final all = loaded.data;
    final conversations = [?all?.bestie, ...?all?.recent];
    final newSolo = selected.value == newSoloChatId;
    final target = conversations
        .where((c) => c.id == selected.value)
        .firstOrNull;
    final canSend = target != null || (newSolo && payload.bestie != null);
    final busy = send.value is _Sending;

    Future<void> submit() async {
      if (!canSend) return;
      // A changed destination or note is a new message, not a retry.
      createdAt.value ??= DateTime.now().millisecondsSinceEpoch ~/ 1000;
      send.value = const _Sending('Sending…');
      try {
        var destination = target;
        if (newSolo) {
          send.value = const _Sending('Starting chat…');
          destination = await createSoloChat(
            progress: solo.value ??= SoloChatProgress(),
            bestie: payload.bestie!,
            note: note.text,
            createdAt: createdAt.value!,
          );
          send.value = const _Sending('Sending…');
        }
        await sendShare(
          payload: payload,
          note: note.text,
          target: destination!,
          createdAt: createdAt.value!,
          uploads: uploads,
          onUploading: (index) => send.value = _Sending(
            payload.attachments.length == 1
                ? 'Uploading ${_noun(payload.attachments[index].kind)}…'
                : 'Uploading ${index + 1} of ${payload.attachments.length}…',
          ),
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
        title: _CommunityTitle(
          payload: payload,
          onSelected: busy ? null : onSwitchCommunity,
        ),
        actions: [
          TextButton(
            onPressed: busy || !canSend ? null : submit,
            child: const Text('Send'),
          ),
        ],
      ),
      body: Column(
        children: [
          if (busy) const LinearProgressIndicator(),
          if (payload.attachments.isNotEmpty)
            _AttachmentPreviews(attachments: payload.attachments),
          Padding(
            padding: const EdgeInsets.all(Grid.xs),
            child: TextField(
              controller: note,
              enabled: !busy,
              minLines: 2,
              maxLines: 5,
              decoration: InputDecoration(
                hintText: payload.attachments.isEmpty
                    ? 'Add a note'
                    : 'Add a caption',
                border: const OutlineInputBorder(),
              ),
            ),
          ),
          if (payload.missed > 0 && payload.problem == null)
            _Notice(
              text: payload.missed == 1
                  ? "1 item can't be sent from here."
                  : "${payload.missed} items can't be sent from here.",
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
              bestieName: payload.bestie?.name,
              onSelect: (id) {
                if (id != selected.value) {
                  createdAt.value = null;
                  solo.value = null;
                }
                selected.value = id;
              },
            ),
          ),
        ],
      ),
    );
  }
}

String _noun(ShareAttachmentKind kind) => switch (kind) {
  ShareAttachmentKind.image => 'photo',
  ShareAttachmentKind.gif => 'GIF',
  ShareAttachmentKind.video => 'video',
  ShareAttachmentKind.file => 'file',
};

/// The community's name; a menu of the others when there are any.
class _CommunityTitle extends StatelessWidget {
  const _CommunityTitle({required this.payload, required this.onSelected});

  final SharePayload payload;
  final ValueChanged<String>? onSelected;

  @override
  Widget build(BuildContext context) {
    final current = payload.community!;
    if (payload.communities.length < 2) return Text(current.name);
    return PopupMenuButton<String>(
      enabled: onSelected != null,
      tooltip: 'Community',
      onSelected: onSelected,
      itemBuilder: (_) => [
        for (final community in payload.communities)
          CheckedPopupMenuItem(
            value: community.id,
            checked: community.id == current.id,
            child: Text(community.name),
          ),
      ],
      child: Row(
        mainAxisSize: MainAxisSize.min,
        children: [
          Flexible(child: Text(current.name, overflow: TextOverflow.ellipsis)),
          const Icon(BuzzIcons.chevronDown, size: 18),
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
