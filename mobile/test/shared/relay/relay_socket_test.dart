import 'dart:async';

import 'package:buzz/shared/relay/relay.dart';
import 'package:flutter_test/flutter_test.dart';
import 'package:web_socket_channel/web_socket_channel.dart';

void main() {
  test('disconnect completes a pending authentication attempt', () async {
    final channel = _ReadyWebSocketChannel();
    final socket = RelaySocket(
      wsUrl: 'wss://relay.example',
      nsec: null,
      onMessage: (_) {},
      onConnected: () => fail('socket must not connect'),
      onDisconnected: (_) {},
      channelFactory: (_) => channel,
    );

    final connecting = socket.connect();
    await Future<void>.delayed(Duration.zero);
    expect(socket.state, SocketState.authenticating);

    await socket.disconnect();
    await connecting.timeout(const Duration(seconds: 1));

    expect(socket.state, SocketState.disconnected);
    expect(channel.closeCount, 1);
  });

  test('auth-phase stream closure reports one disconnection', () async {
    final channel = _ReadyWebSocketChannel();
    final disconnections = <Object?>[];
    final socket = RelaySocket(
      wsUrl: 'wss://relay.example',
      nsec: null,
      onMessage: (_) {},
      onConnected: () => fail('socket must not connect'),
      onDisconnected: disconnections.add,
      channelFactory: (_) => channel,
    );

    final connecting = socket.connect();
    await Future<void>.delayed(Duration.zero);
    expect(socket.state, SocketState.authenticating);

    await channel.closeStream();
    await connecting.timeout(const Duration(seconds: 1));

    expect(disconnections, hasLength(1));
    expect(disconnections.single, isA<Exception>());
    expect(socket.state, SocketState.disconnected);
    expect(channel.closeCount, 1);
  });

  test('auth-phase stream error reports one disconnection', () async {
    final channel = _ReadyWebSocketChannel();
    final disconnections = <Object?>[];
    final socket = RelaySocket(
      wsUrl: 'wss://relay.example',
      nsec: null,
      onMessage: (_) {},
      onConnected: () => fail('socket must not connect'),
      onDisconnected: disconnections.add,
      channelFactory: (_) => channel,
    );

    final connecting = socket.connect();
    await Future<void>.delayed(Duration.zero);
    expect(socket.state, SocketState.authenticating);

    channel.addStreamError(StateError('connection reset'));
    await connecting.timeout(const Duration(seconds: 1));

    expect(disconnections, hasLength(1));
    expect(disconnections.single, isA<StateError>());
    expect(socket.state, SocketState.disconnected);
    expect(channel.closeCount, 1);
  });

  test('hung handshake times out, closes, and reports disconnection', () async {
    final channel = _HungWebSocketChannel();
    final disconnected = Completer<Object?>();
    final socket = RelaySocket(
      wsUrl: 'wss://relay.example',
      nsec: null,
      onMessage: (_) {},
      onConnected: () => fail('socket must not connect'),
      onDisconnected: disconnected.complete,
      channelFactory: (_) => channel,
      connectTimeout: const Duration(milliseconds: 1),
    );

    await socket.connect();

    expect(await disconnected.future, isA<TimeoutException>());
    expect(socket.state, SocketState.disconnected);
    expect(channel.closeCount, 1);
  });
}

class _ReadyWebSocketChannel implements WebSocketChannel {
  final _controller = StreamController<dynamic>();
  int closeCount = 0;

  @override
  Future<void> get ready => Future.value();

  @override
  String? get protocol => null;

  @override
  int? get closeCode => null;

  @override
  String? get closeReason => null;

  @override
  Stream<dynamic> get stream => _controller.stream;

  Future<void> closeStream() => _controller.close();

  void addStreamError(Object error) => _controller.addError(error);

  @override
  late final WebSocketSink sink = _RecordingWebSocketSink(
    _controller.sink,
    () => closeCount++,
  );

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _HungWebSocketChannel implements WebSocketChannel {
  final _controller = StreamController<dynamic>();
  final _ready = Completer<void>();
  int closeCount = 0;

  @override
  Future<void> get ready => _ready.future;

  @override
  String? get protocol => null;

  @override
  int? get closeCode => null;

  @override
  String? get closeReason => null;

  @override
  Stream<dynamic> get stream => _controller.stream;

  @override
  late final WebSocketSink sink = _RecordingWebSocketSink(
    _controller.sink,
    () => closeCount++,
  );

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}

class _RecordingWebSocketSink implements WebSocketSink {
  final void Function() onClose;

  _RecordingWebSocketSink(StreamSink<dynamic> sink, this.onClose);

  @override
  Future<void> close([int? closeCode, String? closeReason]) async {
    onClose();
  }

  @override
  dynamic noSuchMethod(Invocation invocation) => super.noSuchMethod(invocation);
}
