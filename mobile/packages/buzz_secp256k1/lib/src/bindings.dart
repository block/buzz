// Raw bindings to the libsecp256k1 symbols this package uses. Signatures match
// third_party/secp256k1/include at the vendored tag. None of these call back
// into Dart, so all are leaf calls.
@DefaultAsset('package:buzz_secp256k1/src/bindings.dart')
library;

import 'dart:ffi';

final class Secp256k1Context extends Opaque {}

/// `SECP256K1_CONTEXT_NONE`.
const contextNone = 1;

/// Sizes of the opaque structs in secp256k1.h and secp256k1_extrakeys.h.
const pubkeySize = 64;
const xonlyPubkeySize = 64;
const keypairSize = 96;

/// Allocation-free context for operations that do not use secret-dependent
/// precomputation (parsing, verification, ECDH).
@Native<Pointer<Secp256k1Context>>(symbol: 'secp256k1_context_static')
external final Pointer<Secp256k1Context> contextStatic;

@Native<Void Function()>(symbol: 'secp256k1_selftest', isLeaf: true)
external void selftest();

@Native<Void Function(Pointer<Secp256k1Context>)>(
  symbol: 'secp256k1_context_destroy',
  isLeaf: true,
)
external void contextDestroy(Pointer<Secp256k1Context> ctx);

@Native<Pointer<Secp256k1Context> Function(UnsignedInt)>(
  symbol: 'secp256k1_context_create',
  isLeaf: true,
)
external Pointer<Secp256k1Context> contextCreate(int flags);

@Native<Int Function(Pointer<Secp256k1Context>, Pointer<Uint8>)>(
  symbol: 'secp256k1_context_randomize',
  isLeaf: true,
)
external int contextRandomize(
  Pointer<Secp256k1Context> ctx,
  Pointer<Uint8> seed32,
);

@Native<Int Function(Pointer<Secp256k1Context>, Pointer<Uint8>)>(
  symbol: 'secp256k1_ec_seckey_verify',
  isLeaf: true,
)
external int seckeyVerify(Pointer<Secp256k1Context> ctx, Pointer<Uint8> seckey);

@Native<
  Int Function(Pointer<Secp256k1Context>, Pointer<Uint8>, Pointer<Uint8>, Size)
>(symbol: 'secp256k1_ec_pubkey_parse', isLeaf: true)
external int pubkeyParse(
  Pointer<Secp256k1Context> ctx,
  Pointer<Uint8> pubkey,
  Pointer<Uint8> input,
  int inputLen,
);

@Native<
  Int Function(Pointer<Secp256k1Context>, Pointer<Uint8>, Pointer<Uint8>)
>(symbol: 'secp256k1_xonly_pubkey_parse', isLeaf: true)
external int xonlyPubkeyParse(
  Pointer<Secp256k1Context> ctx,
  Pointer<Uint8> pubkey,
  Pointer<Uint8> input32,
);

@Native<
  Int Function(Pointer<Secp256k1Context>, Pointer<Uint8>, Pointer<Uint8>)
>(symbol: 'secp256k1_xonly_pubkey_serialize', isLeaf: true)
external int xonlyPubkeySerialize(
  Pointer<Secp256k1Context> ctx,
  Pointer<Uint8> output32,
  Pointer<Uint8> pubkey,
);

@Native<
  Int Function(Pointer<Secp256k1Context>, Pointer<Uint8>, Pointer<Uint8>)
>(symbol: 'secp256k1_keypair_create', isLeaf: true)
external int keypairCreate(
  Pointer<Secp256k1Context> ctx,
  Pointer<Uint8> keypair,
  Pointer<Uint8> seckey,
);

@Native<
  Int Function(
    Pointer<Secp256k1Context>,
    Pointer<Uint8>,
    Pointer<Int>,
    Pointer<Uint8>,
  )
>(symbol: 'secp256k1_keypair_xonly_pub', isLeaf: true)
external int keypairXonlyPub(
  Pointer<Secp256k1Context> ctx,
  Pointer<Uint8> pubkey,
  Pointer<Int> parity,
  Pointer<Uint8> keypair,
);

@Native<
  Int Function(
    Pointer<Secp256k1Context>,
    Pointer<Uint8>,
    Pointer<Uint8>,
    Pointer<Uint8>,
    Pointer<Uint8>,
  )
>(symbol: 'secp256k1_schnorrsig_sign32', isLeaf: true)
external int schnorrsigSign32(
  Pointer<Secp256k1Context> ctx,
  Pointer<Uint8> sig64,
  Pointer<Uint8> msg32,
  Pointer<Uint8> keypair,
  Pointer<Uint8> auxRand32,
);

@Native<
  Int Function(
    Pointer<Secp256k1Context>,
    Pointer<Uint8>,
    Pointer<Uint8>,
    Size,
    Pointer<Uint8>,
  )
>(symbol: 'secp256k1_schnorrsig_verify', isLeaf: true)
external int schnorrsigVerify(
  Pointer<Secp256k1Context> ctx,
  Pointer<Uint8> sig64,
  Pointer<Uint8> msg,
  int msgLen,
  Pointer<Uint8> pubkey,
);

/// Defined in src/buzz_secp256k1_ecdh.c: `secp256k1_ecdh` returning the raw
/// x-coordinate of the shared point.
@Native<
  Int Function(
    Pointer<Secp256k1Context>,
    Pointer<Uint8>,
    Pointer<Uint8>,
    Pointer<Uint8>,
  )
>(symbol: 'buzz_secp256k1_ecdh_x', isLeaf: true)
external int ecdhX(
  Pointer<Secp256k1Context> ctx,
  Pointer<Uint8> output32,
  Pointer<Uint8> pubkey,
  Pointer<Uint8> seckey32,
);
