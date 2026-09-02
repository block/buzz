#!/bin/bash
# buzz-relay service entrypoint (launchd) — env from repo .env + demo flags
cd /Users/mfethe/buzz-work/buzz-trunk
set -a; source .env; set +a
export BUZZ_AUTO_MIGRATE=true
export BUZZ_RECONCILE_CHANNELS=true
export BUZZ_BIND_ADDR=0.0.0.0:3610
export BUZZ_HEALTH_PORT=8090
export BUZZ_METRICS_PORT=9112
export DATABASE_URL=postgres://buzz:buzz@127.0.0.1:55433/buzz
export REDIS_URL=redis://127.0.0.1:6379
export BUZZ_S3_ENDPOINT=http://127.0.0.1:9000
export BUZZ_REQUIRE_AUTH_TOKEN=false
exec ./target/release/buzz-relay
