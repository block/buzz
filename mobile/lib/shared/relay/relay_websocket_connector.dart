import 'package:web_socket_channel/io.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

/// How often to send a WebSocket ping frame, and how long to wait for the
/// matching pong before declaring the socket dead.
///
/// The relay pings every 30s and closes after 3 missed pongs
/// (`crates/buzz-relay/src/connection.rs`). That protects the relay from dead
/// clients; it does nothing for the client. Mobile networks — cellular in
/// particular — silently drop idle TCP flows without sending a FIN, so an
/// unused socket can stay dead while the client still reports it as open, and
/// nothing surfaces it until some later network event or an app restart.
///
/// `dart:io` never has two pings outstanding: it waits one interval, sends a
/// ping, then allows one more interval for the pong. Detection therefore takes
/// up to `2 * relayPingInterval` — about 40s at this value, which keeps
/// recovery under a minute while the app is foregrounded. The cost is three
/// extra ping/pong exchanges per idle foreground minute; the session already
/// disconnects 5s after being backgrounded
/// ([RelaySessionNotifier], `relay_session.dart`), so this does not run while
/// the app is suspended. Raising it to 30s would halve the probe traffic and
/// roughly double time-to-detection.
const relayPingInterval = Duration(seconds: 20);

/// Opens the relay WebSocket with client-side liveness probing enabled.
///
/// [WebSocketChannel.connect] resolves to a platform-adaptive implementation
/// that exposes no ping configuration, so a half-open socket never surfaces as
/// a close event. [IOWebSocketChannel] does support it: when a ping goes
/// unanswered for [relayPingInterval] the socket closes with `goingAway`,
/// which reaches the existing `onDone` handler and drives the normal reconnect
/// path — no additional lifecycle handling required.
///
/// The mobile app targets Android and iOS only (`mobile/` has no web target),
/// so binding to the `dart:io` implementation is safe here.
WebSocketChannel connectRelayWebSocket(
  Uri uri, {
  Duration pingInterval = relayPingInterval,
}) {
  return IOWebSocketChannel.connect(uri, pingInterval: pingInterval);
}
