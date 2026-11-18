/* NIP-44 needs the raw 32-byte x-coordinate of the ECDH shared point.
 * libsecp256k1's default ECDH hash returns SHA256(compressed point), so this
 * shim passes a hash function that copies x unchanged. It keeps the scalar
 * multiplication on secp256k1_ecdh's constant-time path; the public
 * secp256k1_ec_pubkey_tweak_mul API uses variable-time ecmult and must not be
 * used with secret scalars. */
#define SECP256K1_BUILD
#include <string.h>

#include "../third_party/secp256k1/include/secp256k1.h"
#include "../third_party/secp256k1/include/secp256k1_ecdh.h"

static int copy_x(unsigned char *output, const unsigned char *x32,
                  const unsigned char *y32, void *data) {
  (void)y32;
  (void)data;
  memcpy(output, x32, 32);
  return 1;
}

SECP256K1_API int buzz_secp256k1_ecdh_x(const secp256k1_context *ctx,
                                        unsigned char *output32,
                                        const secp256k1_pubkey *pubkey,
                                        const unsigned char *seckey32) {
  return secp256k1_ecdh(ctx, output32, pubkey, seckey32, copy_x, NULL);
}
