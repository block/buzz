part of '../thread_detail_page.dart';

const _landingHighlightDuration = Duration(seconds: 3);
const _landingHighlightDelay = Duration(milliseconds: 50);
const _landingHighlightTransitionDuration = Duration(milliseconds: 300);
const _landingHighlightOpacity = 0.12;

/// The message to highlight after landing on [messageId]: shown once the
/// target is laid out ([ready]) and the route has arrived, then faded out.
String? _useLandingHighlight(
  BuildContext context,
  String? messageId, {
  required bool ready,
}) {
  final routeAnimation = ModalRoute.of(context)?.animation;
  final reducedMotion = MediaQuery.disableAnimationsOf(context);
  final highlightedMessageId = useState<String?>(null);
  useEffect(() {
    if (messageId == null || !ready) return null;
    var disposed = false;
    Timer? revealTimer;
    Timer? dismissTimer;

    void revealHighlight() {
      WidgetsBinding.instance.addPostFrameCallback((_) {
        if (disposed) return;
        revealTimer = Timer(_landingHighlightDelay, () {
          if (disposed) return;
          highlightedMessageId.value = messageId;
          dismissTimer = Timer(
            _landingHighlightDuration +
                (reducedMotion
                    ? Duration.zero
                    : _landingHighlightTransitionDuration),
            () {
              if (!disposed) highlightedMessageId.value = null;
            },
          );
        });
      });
    }

    void handleRouteStatus(AnimationStatus status) {
      if (status != AnimationStatus.completed) return;
      routeAnimation?.removeStatusListener(handleRouteStatus);
      revealHighlight();
    }

    if (routeAnimation == null ||
        routeAnimation.status == AnimationStatus.completed) {
      revealHighlight();
    } else {
      routeAnimation.addStatusListener(handleRouteStatus);
    }

    return () {
      disposed = true;
      routeAnimation?.removeStatusListener(handleRouteStatus);
      revealTimer?.cancel();
      dismissTimer?.cancel();
    };
  }, [messageId, ready, reducedMotion, routeAnimation]);
  return highlightedMessageId.value;
}
