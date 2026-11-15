#!/bin/bash
# usage: proto-seed.sh RELAY_HTTP_URL  -> prints nsec of receiver, writes /tmp/proto-nsec.txt
cd ~/.buzz/REPOS/buzz-android-push-proto
B=target/debug/buzz; export BUZZ_RELAY_URL=$1
SKA=$(openssl rand -hex 32); SKB=$(openssl rand -hex 32)
BUZZ_PRIVATE_KEY=$SKB $B users set-profile --name "Proto Sender" >/dev/null
CH=$(BUZZ_PRIVATE_KEY=$SKB $B channels create --name proto-chat --type stream --visibility open | python3 -c 'import sys,json;print(json.load(sys.stdin)["channel_id"])')
BUZZ_PRIVATE_KEY=$SKB $B messages send --channel $CH --content "Hello from the headless Dart push prototype" >/dev/null
BUZZ_PRIVATE_KEY=$SKA $B users set-profile --name "Proto Receiver" >/dev/null
BUZZ_PRIVATE_KEY=$SKA $B channels join --channel $CH >/dev/null
python3 - $SKA > /tmp/proto-nsec.txt <<'PY'
import sys
CH="qpzry9x8gf2tvdw0s3jn54khce6mua7l"
def pm(v):
    G=[0x3b6a57b2,0x26508e6d,0x1ea119fa,0x3d4233dd,0x2a1462b3];c=1
    for x in v:
        b=c>>25;c=(c&0x1ffffff)<<5^x
        for i in range(5):
            if (b>>i)&1:c^=G[i]
    return c
hrp="nsec";v=[];acc=0;bits=0
for b in bytes.fromhex(sys.argv[1]):
    acc=(acc<<8)|b;bits+=8
    while bits>=5: bits-=5;v.append((acc>>bits)&31)
if bits: v.append((acc<<(5-bits))&31)
he=[ord(x)>>5 for x in hrp]+[0]+[ord(x)&31 for x in hrp]
p=pm(he+v+[0]*6)^1
print(hrp+"1"+"".join(CH[x] for x in v+[(p>>5*(5-i))&31 for i in range(6)]))
PY
cat /tmp/proto-nsec.txt
