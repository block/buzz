import 'dart:ffi';
import 'dart:io';

import 'package:ffi/ffi.dart';
import 'package:nostr/nostr.dart' as nostr;

/// Signs canonical Nostr events with libsecp256k1 on iOS.
///
/// Native keypairs exist only for this call and are wiped before returning.
/// Other platforms retain the existing Dart implementation, including the
/// optional cached [pubkey]. iOS derives its public key from the native keypair.
nostr.Event signEvent({
  required int kind,
  required String content,
  required String secretKey,
  String? pubkey,
  int? createdAt,
  List<List<String>> tags = const [],
  bool verify = false,
}) {
  if (!Platform.isIOS) {
    return nostr.Event.from(
      kind: kind,
      content: content,
      secretKey: secretKey,
      pubkey: pubkey,
      createdAt: createdAt,
      tags: tags,
      verify: verify,
    );
  }
  final event = _native.sign(
    kind: kind,
    content: content,
    secretKey: secretKey,
    createdAt: createdAt,
    tags: tags,
  );
  return verify ? event.copyWith(verify: true) : event;
}

final _native = NativeEventSigner(DynamicLibrary.process());

/// Native adapter exposed for cross-implementation tests and benchmarks.
class NativeEventSigner {
  /// Loads the three versioned-by-app native entry points. Missing symbols fail
  /// explicitly; iOS must never silently fall back to expensive Dart signing.
  NativeEventSigner(DynamicLibrary library)
    : _create = library
          .lookupFunction<
            Int32 Function(
              Pointer<Uint8>,
              Pointer<Uint8>,
              Pointer<Pointer<Void>>,
            ),
            int Function(Pointer<Uint8>, Pointer<Uint8>, Pointer<Pointer<Void>>)
          >('buzz_signer_create'),
      _sign = library
          .lookupFunction<
            Int32 Function(Pointer<Void>, Pointer<Uint8>, Pointer<Uint8>),
            int Function(Pointer<Void>, Pointer<Uint8>, Pointer<Uint8>)
          >('buzz_signer_sign'),
      _destroy = library
          .lookupFunction<
            Void Function(Pointer<Void>),
            void Function(Pointer<Void>)
          >('buzz_signer_destroy');

  final int Function(Pointer<Uint8>, Pointer<Uint8>, Pointer<Pointer<Void>>)
  _create;
  final int Function(Pointer<Void>, Pointer<Uint8>, Pointer<Uint8>) _sign;
  final void Function(Pointer<Void>) _destroy;

  /// Builds the canonical hash using the same package as the Dart signer.
  nostr.Event sign({
    required int kind,
    required String content,
    required String secretKey,
    int? createdAt,
    List<List<String>> tags = const [],
  }) {
    if (!RegExp(r'^[0-9a-fA-F]{64}$').hasMatch(secretKey)) {
      throw const FormatException('Signing key must be 32 hex bytes');
    }
    final bytes = calloc<Uint8>(160);
    final handle = calloc<Pointer<Void>>();
    try {
      for (var i = 0; i < 32; i++) {
        bytes[i] = int.parse(secretKey.substring(i * 2, i * 2 + 2), radix: 16);
      }
      final publicKey = bytes + 32;
      _check(_create(bytes, publicKey, handle));
      final event = nostr.Event.unsigned(
        pubkey: _hex(publicKey, 32),
        kind: kind,
        content: content,
        createdAt: createdAt,
        tags: tags,
      );
      final message = bytes + 64;
      for (var i = 0; i < 32; i++) {
        message[i] = int.parse(event.id.substring(i * 2, i * 2 + 2), radix: 16);
      }
      final signature = bytes + 96;
      _check(_sign(handle.value, message, signature));
      // Native signing self-verifies. Avoid a second pure-Dart verification.
      return event.copyWith(sig: _hex(signature, 64));
    } finally {
      _destroy(handle.value);
      bytes.asTypedList(160).fillRange(0, 160, 0);
      calloc.free(bytes);
      calloc.free(handle);
    }
  }

  static String _hex(Pointer<Uint8> bytes, int length) => bytes
      .asTypedList(length)
      .map((b) => b.toRadixString(16).padLeft(2, '0'))
      .join();

  static void _check(int status) {
    switch (status) {
      case 0:
        return;
      case 1:
        throw const FormatException('Invalid secp256k1 signing key');
      case 2:
        throw StateError('Native signing random source failed');
      default:
        throw StateError('Native signing failed ($status)');
    }
  }
}
