#include "../vendor/include/secp256k1.h"
#include "../vendor/include/secp256k1_extrakeys.h"
#include "../vendor/include/secp256k1_schnorrsig.h"

/* Short-lived signing handles own a keypair, never an account/session cache. */
typedef struct buzz_signer buzz_signer;
/* 0 success, 1 invalid key, 2 random source, 3 allocation/operation failure. */
int buzz_signer_create(const unsigned char secret[32], unsigned char public_key[32], buzz_signer **out);
int buzz_signer_sign(const buzz_signer *signer, const unsigned char message[32], unsigned char signature[64]);
void buzz_signer_destroy(buzz_signer *signer);
