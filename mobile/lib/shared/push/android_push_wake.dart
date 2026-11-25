import 'dart:io';
import 'dart:ui';

import 'package:flutter/services.dart';
import 'package:flutter/widgets.dart';
import 'package:path_provider/path_provider.dart';

import '../community/community.dart';
import '../hosted/builderlab_account.dart';
import '../http/app_http_client.dart';
import '../hosted/hosted_relay_auth.dart';
import '../hosted/organization_environment.dart';
import '../relay/relay_provider.dart';
import 'push_wake_resolver.dart';
import 'push_wake_runner.dart';
import 'shown_message_store.dart';

/// The channel the native wake handler runs a wake over. The handler starts
/// this engine, waits for [wakeReadyMethod], then asks for each wake with
/// [wakeMethod] and posts the notifications of the answer.
const androidPushWakeChannel = MethodChannel('buzz/push_wake');
const wakeReadyMethod = 'ready';
const wakeMethod = 'wake';

/// The time the native handler has left for a wake's lookup, which it sends
/// as `remainingMillis`, or null when it sends none.
Duration? wakeRemaining(Object? arguments) {
  final millis = arguments is Map ? arguments['remainingMillis'] : null;
  return millis is int ? Duration(milliseconds: millis < 0 ? 0 : millis) : null;
}

/// What a wake sends the native handler.
///
/// The handler may ask for another wake while one runs. [PushWakeRunner] folds
/// it into the run in progress, so only the call that started the run gets
/// the notifications, once. A call that joined it gets [joinedWake], which
/// tells the handler to post nothing for it.
const joinedWake = <String, Object?>{'joined': true};

Map<String, Object?> pushWakeChannelPayload(PushWakeResult result) => {
  'notifications': [
    for (final notification in result.notifications)
      {
        'eventId': notification.eventId,
        'createdAtMillis': notification.createdAt.millisecondsSinceEpoch,
        'title': notification.title,
        'body': notification.body,
        'conversationId': notification.conversationId,
        'conversationTitle': ?notification.conversationTitle,
        'category': notification.category.name,
        'target': notification.target.toMap(),
      },
  ],
  'incomplete': result.incomplete,
};

/// Answers the native handler's wake requests from a [PushWakeRunner].
class PushWakeChannelHandler {
  PushWakeChannelHandler(this._runner);

  final PushWakeRunner _runner;
  Future<PushWakeResult>? _latest;

  /// Runs a wake, or joins the one in progress, within [remaining] when the
  /// handler sends it.
  Future<Map<String, Object?>> wake({Duration? remaining}) {
    final run = _runner.wake(remaining: remaining);
    // The runner hands every caller of one run the same future.
    if (identical(run, _latest)) return run.then((_) => joinedWake);
    _latest = run;
    return run.then(pushWakeChannelPayload);
  }
}

/// Entry point of the background engine the native wake handler starts.
///
/// It runs headless: no UI and no Riverpod state. The foreground app never
/// runs this, since the handler does nothing while the app is in the
/// foreground.
Future<void> runAndroidPushWake() async {
  WidgetsFlutterBinding.ensureInitialized();
  DartPluginRegistrant.ensureInitialized();
  // Each wake runs in a new engine, and a Cronet engine opened in it would
  // outlive it.
  useDartIoHttpOnly();
  final store = await _openShownMessageStore();
  final relayHeaders = _headlessRelayHeaders();
  final handler = PushWakeChannelHandler(
    PushWakeRunner(
      (budget) => resolvePushWake(
        store: store,
        relayHeaders: relayHeaders,
        timeouts: PushWakeTimeouts(total: budget),
      ),
    ),
  );
  androidPushWakeChannel.setMethodCallHandler(
    (call) => switch (call.method) {
      wakeMethod => handler.wake(remaining: wakeRemaining(call.arguments)),
      _ => throw MissingPluginException('Unknown ${call.method}'),
    },
  );
  await androidPushWakeChannel.invokeMethod<void>(wakeReadyMethod);
}

Future<ShownMessageStore> _openShownMessageStore() async {
  final support = await getApplicationSupportDirectory();
  return DirectoryShownMessageStore(
    Directory('${support.path}/shown_messages'),
  );
}

/// Headers for the relays that require the account's federated identity
/// (NIP-FI), read the way the foreground app reads them. The wake is a
/// background caller: it mints its own assertion, never records a failure for
/// the foreground to show, and sends none when it can't get one, so the relay
/// decides.
Future<Map<String, String>> Function(Community) _headlessRelayHeaders() {
  final organizationOrigin = OrganizationEnvironment.origin;
  // The foreground app owns these session records; a wake only reads them.
  final auth = HostedRelayAuth(
    BuilderlabAccountClient(readOnlySession: true),
    organization: organizationOrigin == null
        ? null
        : BuilderlabAccountClient(
            accountOrigin: organizationOrigin,
            readOnlySession: true,
          ),
  );
  return (community) => auth.headers(
    RelayConfig(baseUrl: community.relayUrl, nsec: community.nsec),
    background: true,
  );
}
