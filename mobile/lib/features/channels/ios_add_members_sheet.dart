import 'dart:async';
import 'dart:collection';

import 'package:flutter/material.dart';
import 'package:flutter/services.dart';

import '../../shared/theme/theme.dart';

/// Hosts the native iOS member picker while Flutter owns its data and actions.
class IosAddMembersSheet extends StatefulWidget {
  const IosAddMembersSheet({
    super.key,
    required this.state,
    required this.onAvatar,
    required this.onQuery,
    required this.onToggle,
    required this.onSubmit,
    required this.onRetry,
    required this.onClose,
  });

  final Map<String, Object?> state;
  final Future<Uint8List?> Function(String pubkey, String avatarKey) onAvatar;
  final ValueChanged<String> onQuery;
  final ValueChanged<String> onToggle;
  final VoidCallback onSubmit;
  final VoidCallback onRetry;
  final VoidCallback onClose;

  @override
  State<IosAddMembersSheet> createState() => _IosAddMembersSheetState();
}

class _IosAddMembersSheetState extends State<IosAddMembersSheet> {
  static const _channel = MethodChannel('buzz/add_members_sheet');
  final _session = UniqueKey().toString();
  bool _presented = false;
  bool _closed = false;
  final _avatars = <String, Uint8List>{};
  final _pendingAvatars = <String, Future<Uint8List?>>{};
  final _avatarQueue =
      Queue<
        ({
          String key,
          Future<Uint8List?> Function() load,
          Completer<Uint8List?> result,
        })
      >();
  int _activeAvatars = 0;

  Future<Uint8List?> _avatar(String pubkey, String key) {
    if (_avatars[key] case final bytes?) return Future.value(bytes);
    if (_pendingAvatars[key] case final pending?) return pending;
    // Favor newly visible rows while bounding both work and retained results.
    if (_avatarQueue.length >= 32) {
      final dropped = _avatarQueue.removeFirst();
      _pendingAvatars.remove(dropped.key);
      dropped.result.complete(null);
    }
    final result = Completer<Uint8List?>();
    final load = widget.onAvatar;
    _pendingAvatars[key] = result.future;
    _avatarQueue.add((key: key, load: () => load(pubkey, key), result: result));
    _drainAvatars();
    return result.future;
  }

  void _drainAvatars() {
    while (mounted &&
        !_closed &&
        _activeAvatars < 4 &&
        _avatarQueue.isNotEmpty) {
      final job = _avatarQueue.removeFirst();
      _activeAvatars++;
      unawaited(() async {
        Uint8List? bytes;
        try {
          bytes = await job.load();
          if (mounted && !_closed && bytes != null) {
            if (_avatars.length >= 64) _avatars.remove(_avatars.keys.first);
            _avatars[job.key] = bytes;
          }
        } catch (_) {
          // The native cell keeps its initial when media cannot be rendered.
        } finally {
          _pendingAvatars.remove(job.key);
          _activeAvatars--;
          job.result.complete(mounted && !_closed ? bytes : null);
          _drainAvatars();
        }
      }());
    }
  }

  @override
  void initState() {
    super.initState();
    _channel.setMethodCallHandler((call) async {
      if (!mounted || _closed) return null;
      final args = Map<Object?, Object?>.from(call.arguments as Map);
      if (args['session'] != _session) return null;
      switch (call.method) {
        case 'avatar':
          return _avatar(
            args['pubkey'] as String? ?? '',
            args['avatarKey'] as String? ?? '',
          );
        case 'query':
          widget.onQuery(args['value'] as String? ?? '');
        case 'toggle':
          widget.onToggle(args['pubkey'] as String? ?? '');
        case 'submit':
          widget.onSubmit();
        case 'retry':
          widget.onRetry();
        case 'closed':
          _closed = true;
          widget.onClose();
      }
    });
    WidgetsBinding.instance.addPostFrameCallback((_) => _present());
  }

  Map<String, Object?> get _state => {'session': _session, ...widget.state};

  Future<void> _present() async {
    if (!mounted) return;
    final theme = utilitySurfaceThemeData(Theme.of(context));
    try {
      await _channel.invokeMethod<void>('present', {
        ..._state,
        'dark': theme.brightness == Brightness.dark,
        'pageColor': theme.colorScheme.surface.toARGB32(),
        'rowColor': theme.colorScheme.surfaceContainerHighest.toARGB32(),
      });
      _presented = true;
      if (!mounted) {
        await _channel.invokeMethod<void>('dismiss', {'session': _session});
        return;
      }
      await _update();
    } on PlatformException {
      _failed();
    } on MissingPluginException {
      _failed();
    }
  }

  void _failed() {
    if (!mounted || _closed) return;
    _closed = true;
    ScaffoldMessenger.of(context).showSnackBar(
      const SnackBar(content: Text("Couldn't open Add members. Try again.")),
    );
    widget.onClose();
  }

  Future<void> _update() async {
    if (!_presented || !mounted || _closed) return;
    try {
      await _channel.invokeMethod<void>('update', _state);
    } on PlatformException {
      _failed();
    }
  }

  @override
  void didUpdateWidget(covariant IosAddMembersSheet oldWidget) {
    super.didUpdateWidget(oldWidget);
    unawaited(_update());
  }

  @override
  void dispose() {
    _channel.setMethodCallHandler(null);
    for (final job in _avatarQueue) {
      job.result.complete(null);
    }
    _avatarQueue.clear();
    _avatars.clear();
    unawaited(
      _channel.invokeMethod<void>('dismiss', {'session': _session}).catchError((
        Object _,
      ) {
        // The engine may already have disposed its native presentation.
      }),
    );
    super.dispose();
  }

  @override
  Widget build(BuildContext context) => const SizedBox.shrink();
}
