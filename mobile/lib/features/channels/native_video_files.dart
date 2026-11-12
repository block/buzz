import 'dart:async';
import 'dart:io';

import 'package:http/http.dart' as http;
import 'package:path_provider/path_provider.dart';

import '../../shared/relay/media_auth.dart';
import '../../shared/utils/bounded_fetch.dart';

/// Owns only the selected native gallery video's authenticated temporary file.
/// UIKit releases its player before selection changes or this session closes.
class NativeVideoFiles {
  /// Creates a session with bounded downloads and injectable temporary storage.
  NativeVideoFiles({
    required this.auth,
    required this.client,
    required this.index,
    this.temporaryDirectory = getTemporaryDirectory,
    this.maxBytes = 100 * 1024 * 1024,
    this.stallTimeout = defaultStallTimeout,
    this.maxDuration = defaultMaxFetchDuration,
  });

  final MediaGetAuthService auth;
  final http.Client client;
  final Future<Directory> Function() temporaryDirectory;
  final int maxBytes;
  final Duration stallTimeout, maxDuration;
  int index;
  bool _disposed = false;
  _VideoFileDownload? _download;
  Future<void> _cleanup = Future.value();

  /// Cancels the previous selection and deletes its file before the next load.
  Future<void> select(int value) async {
    if (value == index) return;
    index = value;
    final previous = _download;
    _download = null;
    if (previous != null) _cleanup = previous.dispose();
    await _cleanup;
  }

  /// Downloads the selected video; null exposes Retry in the native viewer.
  Future<String?> load(int value, String url) async {
    await _cleanup;
    if (_disposed || value != index) return null;
    final previous = _download;
    if (previous != null && previous.finished) {
      _download = null;
      _cleanup = previous.dispose();
      await _cleanup;
      if (_disposed || value != index) return null;
    }
    final download = _download ??= _VideoFileDownload(this, url);
    final path = await download.result;
    if (_disposed || value != index || _download != download) return null;
    // A visible Retry starts a fresh request.
    if (path == null) {
      _download = null;
    }
    return path;
  }

  /// Cancels pending IO and deletes the session file after UIKit releases it.
  Future<void> dispose() async {
    _disposed = true;
    final previous = _download;
    _download = null;
    await previous?.dispose();
    await _cleanup;
  }
}

class _VideoFileDownload {
  _VideoFileDownload(this.owner, this.url) {
    result = _fetch().whenComplete(() => finished = true);
  }
  final NativeVideoFiles owner;
  final String url;
  final stop = Completer<void>();
  late final Future<String?> result;
  Directory? directory;
  bool disposed = false;
  bool finished = false;

  Future<String?> _fetch() async {
    Timer? stall;
    void abort() {
      if (!stop.isCompleted) stop.complete();
    }

    void restartStall() {
      stall?.cancel();
      stall = Timer(owner.stallTimeout, abort);
    }

    final deadline = Timer(owner.maxDuration, abort);
    StreamIterator<List<int>>? body;
    RandomAccessFile? output;
    var success = false;
    restartStall();
    try {
      final uri = Uri.parse(url);
      final request = http.AbortableRequest(
        'GET',
        uri,
        abortTrigger: stop.future,
      )..headers.addAll(owner.auth.headersFor(url));
      final response = await owner.client.send(request);
      body = StreamIterator(response.stream);
      if (disposed ||
          stop.isCompleted ||
          response.statusCode != 200 ||
          (response.contentLength ?? 0) > owner.maxBytes) {
        return null;
      }
      final root = await owner.temporaryDirectory();
      directory = await root.createTemp('buzz-gallery-video-');
      if (disposed || stop.isCompleted) return null;
      final extension = uri.path.toLowerCase().endsWith('.mov')
          ? '.mov'
          : '.mp4';
      final file = File('${directory!.path}/video$extension');
      output = await file.open(mode: FileMode.write);
      var size = 0;
      while (await body.moveNext()) {
        restartStall();
        if (disposed || stop.isCompleted) return null;
        final chunk = body.current;
        if (chunk.length > owner.maxBytes - size) return null;
        size += chunk.length;
        await output.writeFrom(
          chunk,
        ); // Backpressure: do not buffer the video in RAM.
      }
      if (disposed || stop.isCompleted || size == 0) return null;
      await output.close();
      output = null;
      success = true;
      return file.path;
    } catch (_) {
      return null; // Native page exposes an accessible Retry action.
    } finally {
      stall?.cancel();
      deadline.cancel();
      abort();
      await body?.cancel();
      try {
        await output?.close();
      } on FileSystemException {
        /* Clean partial file below. */
      }
      if (!success || disposed) await _delete();
    }
  }

  Future<void> _delete() async {
    final value = directory;
    directory = null;
    if (value == null) return;
    try {
      await value.delete(recursive: true);
    } on FileSystemException {
      // OS temporary storage cleanup must not prevent closing the gallery.
    }
  }

  Future<void> dispose() async {
    disposed = true;
    if (!stop.isCompleted) stop.complete();
    await result;
    await _delete();
  }
}
