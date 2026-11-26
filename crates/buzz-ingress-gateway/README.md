# buzz-ingress-gateway

Receives webhooks from providers (GitHub, PagerDuty, Sentry, incident.io),
verifies them, and posts one kind-9 message to each Buzz channel whose
subscription matches.

```text
provider ──POST /webhooks/{integration}──▶ intake: Host check, HMAC, dedupe
                                              │ ingress_gateway_deliveries
                                              ▼
                                           fan-out: CEL filter + message template
                                              │ ingress_gateway_messages
                                              ▼
                                           forwarder: sign kind 9, POST {scheme}://{community_host}/events
```

Integrations, their secrets and subscriptions are written through the
management API on the private listener. Callers sign each request with NIP-98
using a key listed in `BUZZ_INGRESS_RELAY_PUBKEYS`. In production the relay
makes these calls; locally you make them yourself.

## Run it locally

This walkthrough runs a relay, the gateway, and a fake GitHub webhook on one
machine. It takes about ten minutes after the first build.

### Prerequisites

- The repository's dev stack, set up once from the repository root:
  `. ./bin/activate-hermit && just setup`. That starts Postgres
  (`buzz-postgres`), Redis and MinIO in Docker.
- `curl`, `jq` and `openssl`.

Run every command below from the repository root with Hermit active.

### 1. Build

```bash
cargo build -p buzz-relay -p buzz-cli -p buzz-ingress-gateway --example dev_nip98
export GW=/tmp/buzz-gw && mkdir -p $GW
NIP98=./target/debug/examples/dev_nip98 BUZZ=./target/debug/buzz
```

### 2. Start a relay on port 80

The forwarder posts to `{scheme}://{community_host}/events`, and a community
host can't carry a port, so the relay must listen on the default port.

```bash
(set -a && . ./.env && set +a && \
 BUZZ_BIND_ADDR=0.0.0.0:80 RELAY_URL=ws://localhost BUZZ_AUTO_MIGRATE=true BUZZ_PUSH_ENABLED=false \
 nohup ./target/debug/buzz-relay > $GW/relay.log 2>&1 &)
curl -s -H 'Accept: application/nostr+json' http://localhost/ | jq -c '{name, self}'
```

### 3. Create keys

Three throwaway keys: the gateway's own key, which signs the messages; a key
that stands in for the relay on the management API; and a user who owns the
channel.

```bash
for name in gateway relaymgmt owner; do
  $NIP98 keygen > $GW/$name.keys
  sed -n 's/^secret_key=//p' $GW/$name.keys > $GW/$name.sk
  sed -n 's/^public_key=//p' $GW/$name.keys > $GW/$name.pk
done
```

### 4. Create the gateway database and start the gateway

The gateway keeps its own database. Migrations run with the admin role from
`.env`; the gateway runs as a separate role with a generated password.

```bash
ADMIN_URL=$(sed -n 's/^DATABASE_URL=//p' .env | sed 's#/[^/]*$#/buzz_gateway#')
openssl rand -hex 16 > $GW/runtime.password
RUNTIME_URL=$(echo "$ADMIN_URL" | sed "s#//[^@]*@#//gateway_runtime:$(cat $GW/runtime.password)@#")

docker exec buzz-postgres psql -U buzz -c "CREATE DATABASE buzz_gateway" \
  -c "CREATE ROLE gateway_runtime LOGIN PASSWORD '$(cat $GW/runtime.password)'"
DATABASE_URL=$ADMIN_URL BUZZ_INGRESS_RUNTIME_DATABASE_ROLE=gateway_runtime \
  ./target/debug/buzz-ingress-gateway --migrate-only

DATABASE_URL=$RUNTIME_URL \
BUZZ_INGRESS_LISTEN_ADDR=127.0.0.1:8480 BUZZ_INGRESS_PRIVATE_ADDR=127.0.0.1:8481 \
BUZZ_INGRESS_SIGNING_KEY=$(cat $GW/gateway.sk) \
BUZZ_INGRESS_SECRET_KEYRING="k1:$(head -c 32 /dev/urandom | base64)" \
BUZZ_INGRESS_RELAY_PUBKEYS=$(cat $GW/relaymgmt.pk) \
BUZZ_INGRESS_RELAY_SCHEME=http BUZZ_INGRESS_PUBLIC_SCHEME=http RUST_LOG=info \
  nohup ./target/debug/buzz-ingress-gateway > $GW/gateway.log 2>&1 &
sleep 2 && curl -s http://127.0.0.1:8481/_readiness
```

Port 8480 takes webhooks. Port 8481 serves the management API and health
checks; keep it private.

### 5. Create a channel

```bash
export BUZZ_RELAY_URL=http://localhost
CHANNEL=$(BUZZ_PRIVATE_KEY=$(cat $GW/owner.sk) $BUZZ --format json channels create \
  --name webhook-test --visibility open --type stream | jq -r '.channel_id // .id')
echo $CHANNEL
```

Messages are signed by the gateway's key. The relay lets anyone in the
community post to an open channel; for a private channel, add the gateway's
public key as a member first.

### 6. Create an integration, its secret and a subscription

`mgmt` signs one management call with the relay-management key:

```bash
mgmt() { # METHOD PATH [JSON]
  local url=http://127.0.0.1:8481$2 body=$GW/body.json
  if [ -n "$3" ]; then
    printf '%s' "$3" > $body
    curl -s -X $1 -H 'Content-Type: application/json' \
      -H "Authorization: $($NIP98 sign $(cat $GW/relaymgmt.sk) $1 $url $body)" --data-binary @$body $url
  else
    curl -s -X $1 -H "Authorization: $($NIP98 sign $(cat $GW/relaymgmt.sk) $1 $url)" $url
  fi; echo
}

INTEGRATION=$(uuidgen | tr A-Z a-z)
openssl rand -hex 20 > $GW/github.secret
mgmt PUT /v1/integrations/$INTEGRATION \
  "{\"operation_id\":\"$(uuidgen | tr A-Z a-z)\",\"revision\":1,\"community_host\":\"localhost\",\"provider\":\"github\"}"
mgmt PUT /v1/integrations/$INTEGRATION/secret \
  "{\"operation_id\":\"$(uuidgen | tr A-Z a-z)\",\"revision\":2,\"secret\":\"$(cat $GW/github.secret)\",\"grace_seconds\":0}"
mgmt PUT /v1/integrations/$INTEGRATION/subscriptions/$(uuidgen | tr A-Z a-z) "$(jq -nc --arg c $CHANNEL '{
  revision: 1, channel_id: $c,
  filter: "event.type == \"pull_request.opened\"",
  message_template: "\"PR #\" + string(payload.number) + \" opened: \" + payload.pull_request.title"}')"
```

`filter` and `message_template` are CEL over `event` (`provider` and `type`,
for example `pull_request.opened`) and `payload` (the webhook body). Both are
optional. Without a filter, every delivery matches. Without a template, the
message is `<provider>: <type>`.

### 7. Send a webhook

This signs a body the way GitHub does. The `Host` header must equal the
integration's `community_host`, so send it to `localhost`, not `127.0.0.1`.

```bash
BODY='{"action":"opened","number":42,"pull_request":{"title":"Fix the build"}}'
curl -s -X POST http://localhost:8480/webhooks/$INTEGRATION \
  -H 'Content-Type: application/json' \
  -H 'X-GitHub-Event: pull_request' -H "X-GitHub-Delivery: $(uuidgen)" \
  -H "X-Hub-Signature-256: sha256=$(printf '%s' "$BODY" | openssl dgst -sha256 -hmac "$(cat $GW/github.secret)" -hex | awk '{print $NF}')" \
  --data-binary "$BODY"; echo
```

The gateway answers `202` with `"state":"queued"`. A repeated delivery ID
answers `duplicate`; a bad signature answers `401`.

### 8. Check the result

```bash
BUZZ_PRIVATE_KEY=$(cat $GW/owner.sk) $BUZZ --format compact messages get --channel $CHANNEL --limit 5
mgmt GET /v1/integrations/$INTEGRATION/deliveries | jq '.deliveries[0] | {state, messages}'
```

The channel shows `PR #42 opened: Fix the build`. The delivery reports
`messages: {queued, accepted, failed}`. A message the relay refused, or whose
template failed, is counted as failed; its error is in
`ingress_gateway_messages.last_error`:

```bash
docker exec buzz-postgres psql -U buzz -d buzz_gateway \
  -c "SELECT subscription_id, state, attempts, content, last_error FROM ingress_gateway_messages"
```

### Other providers

PagerDuty integrations use `"provider":"pagerduty"`. Sign the body with
`X-PagerDuty-Signature: v1=<hex HMAC-SHA256>`. The delivery ID is
`event.id`, and the event type is `event.event_type`, for example
`incident.triggered`.

## Limits

- **One identity signs every message.** `BUZZ_INGRESS_SIGNING_KEY` authors
  all messages, through the placeholder `GatewayKeySigner` in `src/signer.rs`.
- **No federated identity.** A community that enforces NIP-FI refuses the
  gateway's requests with `401`, because the gateway sends no
  `Nostr-Federated-Identity` assertion.
- **Buzz 1.0 needs TLS.** The Buzz 1.0 desktop app only connects to
  `wss://` relays. To watch messages there, put TLS in front of the relay:
  - a certificate for `localhost` that your OS trusts;
  - a proxy on port 443 to port 80;
  - the relay restarted with `RELAY_URL=wss://localhost`;
  - the gateway restarted with `BUZZ_INGRESS_RELAY_SCHEME=https`.

  The relay derives the NIP-42 and NIP-98 URLs it expects from `RELAY_URL`, so
  the gateway has to use `https` too.

## Clean up

```bash
pkill -f target/debug/buzz-ingress-gateway; pkill -f target/debug/buzz-relay
docker exec buzz-postgres psql -U buzz -c "DROP DATABASE buzz_gateway" -c "DROP ROLE gateway_runtime"
rm -rf /tmp/buzz-gw
```
