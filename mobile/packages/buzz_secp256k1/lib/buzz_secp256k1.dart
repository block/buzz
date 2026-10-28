/// BIP-340 Schnorr signatures and ECDH over secp256k1, backed by
/// bitcoin-core/libsecp256k1.
library;

import 'dart:ffi';
import 'dart:math';
import 'dart:typed_data';

import 'package:ffi/ffi.dart';

import 'src/bindings.dart' as c;

// Per-isolate native state. Isolates are single-threaded and every binding is
// a leaf call, so a call can never observe another call's scratch state. Both
// allocations are released by native finalizers when the isolate exits, so
// short-lived `Isolate.run` workers do not leak.
final class _State implements Finalizable {
  _State() {
    _scratchFinalizer.attach(this, scratch.cast());
  }

  static final _scratchFinalizer = NativeFinalizer(malloc.nativeFree);
  static final _contextFinalizer = NativeFinalizer(
    Native.addressOf<
          NativeFunction<Void Function(Pointer<c.Secp256k1Context>)>
        >(c.contextDestroy)
        .cast(),
  );

  final Pointer<Uint8> scratch = malloc<Uint8>(_scratchSize);

  /// Randomized context for signing and key derivation, created on first use.
  late final Pointer<c.Secp256k1Context> signing = _createSigningContext();

  Pointer<c.Secp256k1Context> _createSigningContext() {
    final ctx = c.contextCreate(c.contextNone);
    if (ctx == nullptr) throw StateError('secp256k1_context_create failed');
    _contextFinalizer.attach(this, ctx.cast());
    // Randomizing the context blinds signing and key generation against side
    // channels, as recommended by secp256k1.h.
    _load(_aux, _secureRandom(32), 32, 'seed');
    try {
      if (c.contextRandomize(ctx, _at(_aux)) != 1) {
        throw StateError('secp256k1_context_randomize failed');
      }
    } finally {
      _wipe(_aux, 32);
    }
    return ctx;
  }
}

final _State _state = _State();

/// Static context for verification and ECDH. secp256k1.h recommends running
/// the self test before relying on it.
final Pointer<c.Secp256k1Context> _verify = () {
  c.selftest();
  return c.contextStatic;
}();

const _sig = 0; // 64 bytes
const _msg = 64; // 32 bytes, also used for 32-byte outputs
const _seckey = 96; // 32 bytes
const _aux = 128; // 32 bytes
const _input = 160; // up to 33 bytes of serialized public key
const _pubkey = 208; // secp256k1_pubkey or secp256k1_xonly_pubkey (64)
const _keypair = 272; // secp256k1_keypair (96)
const _parity = 368; // int
const _scratchSize = 384;

Uint8List _secureRandom(int length) {
  final rng = Random.secure();
  return Uint8List.fromList(List.generate(length, (_) => rng.nextInt(256)));
}

Pointer<Uint8> _at(int offset) => _state.scratch + offset;

void _load(int offset, Uint8List bytes, int length, String name) {
  if (bytes.length != length) {
    throw ArgumentError.value(bytes.length, name, 'must be $length bytes');
  }
  _at(offset).asTypedList(length).setAll(0, bytes);
}

Uint8List _read(int offset, int length) =>
    Uint8List.fromList(_at(offset).asTypedList(length));

void _wipe(int offset, int length) =>
    _at(offset).asTypedList(length).fillRange(0, length, 0);

/// Verifies a 64-byte BIP-340 [signature] over a 32-byte [message] for the
/// 32-byte x-only [publicKey].
///
/// Returns false when the signature is invalid or [publicKey] is not on the
/// curve. Throws [ArgumentError] when an input has the wrong length.
bool schnorrVerify({
  required Uint8List signature,
  required Uint8List message,
  required Uint8List publicKey,
}) {
  _load(_sig, signature, 64, 'signature');
  _load(_msg, message, 32, 'message');
  _load(_input, publicKey, 32, 'publicKey');
  if (c.xonlyPubkeyParse(_verify, _at(_pubkey), _at(_input)) != 1) return false;
  return c.schnorrsigVerify(_verify, _at(_sig), _at(_msg), 32, _at(_pubkey)) ==
      1;
}

/// Signs a 32-byte [message] with the 32-byte [secretKey] and returns a
/// 64-byte BIP-340 signature.
///
/// [auxRand] defaults to 32 fresh random bytes, as BIP-340 recommends. Throws
/// [ArgumentError] when [secretKey] is zero or not below the curve order.
Uint8List schnorrSign({
  required Uint8List message,
  required Uint8List secretKey,
  Uint8List? auxRand,
}) {
  // Create the signing context first: its one-time randomization uses the
  // aux scratch slot, which would otherwise clobber [auxRand].
  final ctx = _state.signing;
  _load(_msg, message, 32, 'message');
  _load(_aux, auxRand ?? _secureRandom(32), 32, 'auxRand');
  _load(_seckey, secretKey, 32, 'secretKey');
  try {
    if (c.keypairCreate(ctx, _at(_keypair), _at(_seckey)) != 1) {
      throw ArgumentError('Invalid secp256k1 secret key');
    }
    if (c.schnorrsigSign32(
          ctx,
          _at(_sig),
          _at(_msg),
          _at(_keypair),
          _at(_aux),
        ) !=
        1) {
      throw StateError('secp256k1_schnorrsig_sign32 failed');
    }
    return _read(_sig, 64);
  } finally {
    _wipe(_seckey, 32);
    _wipe(_keypair, c.keypairSize);
    _wipe(_aux, 32);
  }
}

/// Returns the 32-byte x-only public key for the 32-byte [secretKey].
///
/// Throws [ArgumentError] when [secretKey] is zero or not below the curve
/// order.
Uint8List xOnlyPublicKey(Uint8List secretKey) {
  final ctx = _state.signing;
  _load(_seckey, secretKey, 32, 'secretKey');
  try {
    if (c.keypairCreate(ctx, _at(_keypair), _at(_seckey)) != 1) {
      throw ArgumentError('Invalid secp256k1 secret key');
    }
    final parity = _at(_parity).cast<Int>();
    if (c.keypairXonlyPub(ctx, _at(_pubkey), parity, _at(_keypair)) != 1 ||
        c.xonlyPubkeySerialize(ctx, _at(_msg), _at(_pubkey)) != 1) {
      throw StateError('secp256k1 x-only public key derivation failed');
    }
    return _read(_msg, 32);
  } finally {
    _wipe(_seckey, 32);
    _wipe(_keypair, c.keypairSize);
  }
}

/// Computes the ECDH shared point between the 32-byte [secretKey] and the
/// 32-byte x-only [publicKey] (lifted to even Y) and returns its raw 32-byte
/// x-coordinate, as NIP-44 requires.
///
/// Throws [ArgumentError] when [publicKey] is not on the curve or [secretKey]
/// is invalid.
Uint8List ecdhX({required Uint8List secretKey, required Uint8List publicKey}) {
  if (publicKey.length != 32) {
    throw ArgumentError.value(
      publicKey.length,
      'publicKey',
      'must be 32 bytes',
    );
  }
  _at(_input).value = 0x02;
  _load(_input + 1, publicKey, 32, 'publicKey');
  if (c.pubkeyParse(_verify, _at(_pubkey), _at(_input), 33) != 1) {
    throw ArgumentError('Invalid secp256k1 public key');
  }
  _load(_seckey, secretKey, 32, 'secretKey');
  try {
    if (c.ecdhX(_verify, _at(_msg), _at(_pubkey), _at(_seckey)) != 1) {
      throw ArgumentError('Invalid secp256k1 secret key');
    }
    return _read(_msg, 32);
  } finally {
    _wipe(_seckey, 32);
    _wipe(_msg, 32);
  }
}
