part of '../thread_detail_page.dart';

/// The thread title's way up one level. [icon] and [label] name the
/// conversation the thread lives in, the same for a sub-thread as for a
/// top-level thread; [onPressed] goes up, to the parent thread or to that
/// conversation, and [semanticLabel] says which. A null [onPressed] is a
/// parent thread that is still loading.
typedef _ThreadUpLevel = ({
  IconData? icon,
  String label,
  String semanticLabel,
  VoidCallback? onPressed,
});

/// The head each open thread page shows, by its route, so a sub-thread can
/// tell when its parent thread is the page directly beneath it.
final _openThreadHeads = Expando<String>();

/// Resolves where "up" leads from [page]: the thread whose head is the
/// direct parent of this page's head, or the conversation for a top-level
/// thread. A parent message that is not loaded is fetched once the thread's
/// own replies have settled without it, and the title waits, inert, until
/// then. Null while the thread's conversation is unresolved.
_ThreadUpLevel? _useThreadUpLevel(
  BuildContext context,
  WidgetRef ref,
  ThreadDetailPage page,
  List<TimelineMessage> messages,
  String? currentPubkey, {
  required bool repliesSettled,
}) {
  final head = page.threadHead;
  final channelId = page.channelId;
  final channel = ref.watch(channelByIdProvider(channelId));
  final conversationName = channel == null
      ? null
      : _threadConversationName(ref, channel, currentPubkey);
  final legacySoloChannels = ref.watch(
    meGroupsProvider.select((g) => g.value?.channels ?? const <String>[]),
  );
  final route = ModalRoute.of(context);
  useEffect(() {
    if (route != null) _openThreadHeads[route] = head.id;
    return null;
  }, [route, head.id]);

  final parentId = head.parentId;
  final loadedParent = parentId == null
      ? null
      : messages.where((message) => message.id == parentId).firstOrNull;
  // The lookup below stops pinning the parent once it arrives.
  final knownParent = useRef<TimelineMessage?>(null);
  if (loadedParent != null) knownParent.value = loadedParent;
  final parent = parentId == null ? null : loadedParent ?? knownParent.value;
  final lookupSettled = useState(false);
  final needsLookup = parentId != null && parent == null && repliesSettled;
  useEffect(() {
    if (!needsLookup) return null;
    final notifier = ref.read(channelMessagesProvider(channelId).notifier);
    var cancelled = false;
    Future<void> lookUp() async {
      try {
        await notifier.loadEventsById({parentId});
      } catch (error) {
        debugPrint('thread: failed to load parent message: $error');
      }
      // A parent that cannot be loaded leaves the conversation as the way up.
      if (!cancelled) lookupSettled.value = true;
    }

    unawaited(lookUp());
    return () {
      cancelled = true;
      notifier.releaseDeepLinkEvents({parentId});
    };
  }, [needsLookup, parentId, channelId]);

  if (channel == null || conversationName == null) return null;

  void open(Route<void> Function() target, bool Function(Route<dynamic>) isIt) {
    // A second tap lands after this page has already left the top.
    if (route == null || !route.isCurrent) return;
    final navigator = Navigator.of(context);
    final below = appRouteObserver.routeBelow(route);
    if (below != null && isIt(below)) {
      unawaited(navigator.maybePop());
    } else {
      unawaited(navigator.push(target()));
    }
  }

  void openConversation() {
    final rootId = head.rootId ?? head.id;
    final routes = ref.read(openChannelRoutesProvider);
    open(
      () => MaterialPageRoute<void>(
        builder: (_) =>
            ChannelDetailPage(channel: channel, initialMessageId: rootId),
      ),
      (below) {
        if (!routes.shows(channelId, below)) return false;
        // An Inbox or push landing left the channel at its newest message.
        routes.reveal(below, rootId);
        return true;
      },
    );
  }

  void openParentThread(TimelineMessage parent) => open(
    () => MaterialPageRoute<void>(
      builder: (_) => ThreadDetailPage(
        threadHead: parent,
        allMessages: messages,
        channelId: channelId,
        currentPubkey: currentPubkey,
        isMember: page.isMember,
        isArchived: page.isArchived,
        initialMessageId: head.id,
      ),
    ),
    (below) => _openThreadHeads[below] == parent.id,
  );

  // The Messages list's glyph stands where a "#" would be written. A DM has
  // none there, only its people's avatars, so it is named in words.
  final icon = conversationListIcon(
    channel,
    legacySoloChannels: legacySoloChannels,
  );
  final label = icon == null ? 'DM · $conversationName' : conversationName;
  // A sub-thread's title looks the same as its thread's; only what it says
  // aloud tells where a tap leads.
  if (parent != null) {
    return (
      icon: icon,
      label: label,
      semanticLabel: 'Go to parent thread',
      onPressed: () => openParentThread(parent),
    );
  }
  if (parentId != null && !lookupSettled.value) {
    return (
      icon: icon,
      label: label,
      semanticLabel: 'Loading parent thread',
      onPressed: null,
    );
  }
  return (
    icon: icon,
    label: label,
    semanticLabel: 'Go to $label',
    onPressed: openConversation,
  );
}
