#include "include/CSecp256k1.h"
#include <Security/Security.h>
#include <pthread.h>
#include <stdlib.h>
#include <string.h>

struct buzz_signer { secp256k1_keypair pair; };
static pthread_once_t context_once = PTHREAD_ONCE_INIT;
static secp256k1_context *context;
static int context_status = 3;

static void wipe(void *buffer, size_t count) {
    volatile unsigned char *bytes = buffer;
    while (count--) *bytes++ = 0;
}

/* Published once, then immutable. libsecp permits concurrent const operations. */
static void initialize_context(void) {
    unsigned char seed[32];
    if (SecRandomCopyBytes(kSecRandomDefault, sizeof(seed), seed) != errSecSuccess) {
        wipe(seed, sizeof(seed));
        context_status = 2;
        return;
    }
    context = secp256k1_context_create(SECP256K1_CONTEXT_NONE);
    if (context && secp256k1_context_randomize(context, seed)) context_status = 0;
    wipe(seed, sizeof(seed));
}

__attribute__((visibility("default"), used))
int buzz_signer_create(const unsigned char secret[32], unsigned char public_key[32], buzz_signer **out) {
    if (!secret || !public_key || !out) return 3;
    *out = NULL;
    pthread_once(&context_once, initialize_context);
    if (context_status) return context_status;
    buzz_signer *signer = calloc(1, sizeof(*signer));
    if (!signer) return 3;
    if (!secp256k1_keypair_create(context, &signer->pair, secret)) {
        buzz_signer_destroy(signer);
        return 1;
    }
    secp256k1_xonly_pubkey key;
    if (!secp256k1_keypair_xonly_pub(context, &key, NULL, &signer->pair) ||
        !secp256k1_xonly_pubkey_serialize(context, public_key, &key)) {
        buzz_signer_destroy(signer);
        return 3;
    }
    *out = signer;
    return 0;
}

__attribute__((visibility("default"), used))
int buzz_signer_sign(const buzz_signer *signer, const unsigned char message[32], unsigned char signature[64]) {
    if (!signer || !message || !signature) return 3;
    unsigned char aux[32];
    if (SecRandomCopyBytes(kSecRandomDefault, sizeof(aux), aux) != errSecSuccess) {
        wipe(aux, sizeof(aux));
        return 2;
    }
    secp256k1_xonly_pubkey key;
    int ok = secp256k1_schnorrsig_sign32(context, signature, message, &signer->pair, aux) &&
        secp256k1_keypair_xonly_pub(context, &key, NULL, &signer->pair) &&
        secp256k1_schnorrsig_verify(context, signature, message, 32, &key);
    wipe(aux, sizeof(aux));
    if (!ok) { wipe(signature, 64); return 3; }
    return 0;
}

__attribute__((visibility("default"), used))
void buzz_signer_destroy(buzz_signer *signer) {
    if (!signer) return;
    wipe(signer, sizeof(*signer));
    free(signer);
}
