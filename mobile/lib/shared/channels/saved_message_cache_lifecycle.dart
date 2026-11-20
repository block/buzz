import 'dart:async';
import 'dart:ffi' as ffi;
import 'dart:io';

import 'package:flutter/services.dart';

/// Copy this lease to the serial cache worker along with the operation.
typedef SavedMessageCacheLease = ({int id, int workerControl});

typedef _NativeControl =
    ffi.Int32 Function(ffi.Int64, ffi.Int32, ffi.IntPtr, ffi.IntPtr);
typedef _DartControl = int Function(int, int, int, int);
typedef _NativeInterrupt = ffi.Void Function(ffi.Pointer<ffi.Void>);

// Resolve the interrupt function in the same native asset as Database.handle.
@ffi.Native<_NativeInterrupt>(
  assetId: 'package:sqlite3/src/ffi/libsqlite3.g.dart',
  symbol: 'sqlite3_interrupt',
)
external void _sqliteInterrupt(ffi.Pointer<ffi.Void> handle);

abstract final class SavedMessageCacheLifecycle {
  static const _channel = MethodChannel('buzz/saved_message_cache');
  static final StreamController<void> _foreground = StreamController.broadcast(
    onListen: () => _channel.setMethodCallHandler((call) async {
      if (call.method == 'foreground') _foreground.add(null);
    }),
    onCancel: () => _channel.setMethodCallHandler(null),
  );

  /// A transition that allows retrying a disk read deferred during launch.
  static Stream<void> get foreground =>
      Platform.isIOS ? _foreground.stream : const Stream.empty();

  /// On the main isolate, acquire before dispatching work to the worker.
  /// Null means that the native scene is inactive or an assertion is unavailable.
  static Future<SavedMessageCacheLease?> acquire() async {
    if (!Platform.isIOS) return (id: 0, workerControl: 0);
    final lease = await _channel.invokeListMethod<int>('acquire');
    if (lease == null) return null;
    return (id: lease[0], workerControl: lease[1]);
  }

  /// On the main isolate, release after the worker acknowledges closing.
  static Future<void> release(SavedMessageCacheLease lease) async {
    if (lease.id != 0) await _channel.invokeMethod<void>('release', lease.id);
  }

  /// On the worker, call before SQLite open or filesystem work.
  static bool beginWorker(SavedMessageCacheLease lease) =>
      lease.id == 0 || _control(lease, 0) != 0;

  /// Register before executing SQL. A rejected registration requires close.
  static bool registerHandle(
    SavedMessageCacheLease lease,
    ffi.Pointer<void> handle,
  ) =>
      lease.id == 0 ||
      _control(
            lease,
            1,
            handle.address,
            ffi.Native.addressOf<ffi.NativeFunction<_NativeInterrupt>>(
              _sqliteInterrupt,
            ).address,
          ) !=
          0;

  /// Call between statements or chunks. An interrupt can miss idle SQLite.
  static bool isCancelled(SavedMessageCacheLease lease) =>
      lease.id != 0 && _control(lease, 2) != 0;

  /// Always detach before closing so native cancellation cannot race close.
  static void detachHandle(SavedMessageCacheLease lease) {
    if (lease.id != 0) _control(lease, 3);
  }

  /// Acknowledge cleanup after close, including after a failed SQLite open.
  static void finishWorker(SavedMessageCacheLease lease) {
    if (lease.id != 0) _control(lease, 4);
  }

  static int _control(
    SavedMessageCacheLease lease,
    int operation, [
    int handle = 0,
    int interrupt = 0,
  ]) => ffi.Pointer<ffi.NativeFunction<_NativeControl>>.fromAddress(
    lease.workerControl,
  ).asFunction<_DartControl>()(lease.id, operation, handle, interrupt);
}
