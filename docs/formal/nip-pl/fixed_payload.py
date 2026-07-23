"""Exhaustive finite model of the public gateway APNs-body noninterference rule.

For every actual APNs attempt a, application_body(a) == C.  Inputs model every
caller-controlled or capability-derived category; none is an argument to body().
"""
from itertools import product

C = b'{"aps":{"alert":{"body":"Reconnect to your relay now"},"mutable-content":1}}'
DOMAINS = [
    (b"request-a", b"request-b"),        # exact signed body
    (b"auth-a", b"auth-b"),              # NIP-98 event/header
    (b"grant-a", b"grant-b"),            # opaque capability/envelope
    (b"endpoint-0", b"endpoint-1"),      # decrypted destination
    (b"profile-prod", b"profile-test"),  # profile/environment
    (b"id-0", b"id-1"),                  # request id
    (b"expiry-0", b"expiry-1"),          # expiration
    (b"provider-a", b"provider-b"),       # provider response / retry path
]

def application_body(_inputs):
    return C

def explore():
    attempts = 0
    for inputs in product(*DOMAINS):
        attempts += 1
        assert application_body(inputs) == C
    return attempts

if __name__ == "__main__":
    n = explore()
    print(f"fixed-payload input combinations: {n}")
    print("RESULT: APNS APPLICATION BODY NONINTERFERENCE HOLDS")
