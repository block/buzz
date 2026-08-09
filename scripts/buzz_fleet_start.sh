#!/usr/bin/env bash
# buzz_fleet_start.sh — Start all fleet coordination processes, detached.
#
# Starts the relay, listener, responder, and optional bridges as fully
# detached processes (setsid + nohup) that survive shell session cleanup.
#
# Usage:
#   ./scripts/buzz_fleet_start.sh --agent devin-local
#   ./scripts/buzz_fleet_start.sh --agent devin-local --actions-config /path/to/actions.yaml
#   ./scripts/buzz_fleet_start.sh --agent devin-local --with-eks-bridge
#   ./scripts/buzz_fleet_start.sh --agent devin-local --with-cloud-bridge
#   ./scripts/buzz_fleet_start.sh --agent devin-local --all
#   ./scripts/buzz_fleet_start.sh --status
#   ./scripts/buzz_fleet_start.sh --stop
#
# Environment:
#   BUZZ_RELAY_URL  — relay URL (default: http://127.0.0.1:3030)
#   FLEET_KEYS      — path to .fleet_keys.env (default: <repo>/.fleet_keys.env)
#   FLEET_CHANNELS  — path to .fleet_channels.env (default: <repo>/.fleet_channels.env)
#   GATEWAY_URL     — OTOFLO Gateway URL for EKS bridge (required with --with-eks-bridge)
#   GATEWAY_TOKEN   — OTOFLO Gateway token (required with --with-eks-bridge)
#   DEVIN_API_KEY   — Devin API key for cloud bridge (required with --with-cloud-bridge)
#   DEVIN_ORG_ID    — Devin org ID for cloud bridge (required with --with-cloud-bridge)
set -euo pipefail

SCRIPT_DIR="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"
REPO_ROOT="$(cd "$SCRIPT_DIR/.." && pwd)"

AGENT=""
ACTIONS_CONFIG=""
WITH_EKS=0
WITH_CLOUD=0
ACTION="start"

while [[ $# -gt 0 ]]; do
    case "$1" in
        --agent) AGENT="$2"; shift 2 ;;
        --actions-config) ACTIONS_CONFIG="$2"; shift 2 ;;
        --with-eks-bridge) WITH_EKS=1; shift ;;
        --with-cloud-bridge) WITH_CLOUD=1; shift ;;
        --all) WITH_EKS=1; WITH_CLOUD=1; shift ;;
        --status) ACTION="status"; shift ;;
        --stop) ACTION="stop"; shift ;;
        --help|-h)
            head -20 "$0"
            exit 0
            ;;
        *) echo "Unknown option: $1"; exit 1 ;;
    esac
done

# Load fleet keys and channels
KEYS_FILE="${FLEET_KEYS:-$REPO_ROOT/.fleet_keys.env}"
CHANNELS_FILE="${FLEET_CHANNELS:-$REPO_ROOT/.fleet_channels.env}"

if [[ ! -f "$KEYS_FILE" ]]; then
    echo "ERROR: Fleet keys not found at $KEYS_FILE"
    echo "Run scripts/buzz_fleet_setup.sh first to generate keys."
    exit 1
fi
if [[ ! -f "$CHANNELS_FILE" ]]; then
    echo "ERROR: Fleet channels not found at $CHANNELS_FILE"
    echo "Run scripts/buzz_fleet_setup.sh first to create channels."
    exit 1
fi

source "$KEYS_FILE"
source "$CHANNELS_FILE"

export BUZZ_RELAY_URL="${BUZZ_RELAY_URL:-http://127.0.0.1:3030}"
BUZZ_CLI="${BUZZ_CLI_PATH:-$REPO_ROOT/target/release/buzz}"

# --- Status ---
if [[ "$ACTION" == "status" ]]; then
    echo "=== Fleet Process Status ==="
    for name in relay listener responder eks_bridge cloud_bridge; do
        pid_file="/tmp/buzz-${AGENT}-${name//_/-}.pid"
        [[ "$name" == "relay" ]] && pid_file="/tmp/buzz-relay.pid"
        [[ "$name" == "eks_bridge" ]] && pid_file="/tmp/buzz-eks-bridge.pid"
        [[ "$name" == "cloud_bridge" ]] && pid_file="/tmp/buzz-cloud-bridge.pid"
        if [[ -f "$pid_file" ]]; then
            pid=$(cat "$pid_file")
            if ps -p "$pid" -o state= >/dev/null 2>&1; then
                state=$(ps -p "$pid" -o state= 2>/dev/null)
                elapsed=$(ps -p "$pid" -o etime= 2>/dev/null)
                echo "  $name: PID=$pid state=$state elapsed=$elapsed"
            else
                echo "  $name: PID=$pid DOWN (stale pid file)"
            fi
        else
            echo "  $name: not started"
        fi
    done
    echo ""
    echo "=== Action Queue ==="
    queue_file="/tmp/buzz-${AGENT}-action-queue.jsonl"
    if [[ -f "$queue_file" ]]; then
        wc -l < "$queue_file" | xargs echo "  pending actions:"
    else
        echo "  (empty)"
    fi
    exit 0
fi

# --- Stop ---
if [[ "$ACTION" == "stop" ]]; then
    echo "Stopping fleet processes for $AGENT..."
    for pid_file in \
        "/tmp/buzz-${AGENT}-listener.pid" \
        "/tmp/buzz-${AGENT}-responder.pid" \
        "/tmp/buzz-eks-bridge.pid" \
        "/tmp/buzz-cloud-bridge.pid" \
        "/tmp/gateway-portforward.pid"; do
        if [[ -f "$pid_file" ]]; then
            pid=$(cat "$pid_file")
            if ps -p "$pid" >/dev/null 2>&1; then
                kill "$pid" 2>/dev/null || true
                echo "  stopped PID=$pid ($pid_file)"
            fi
            rm -f "$pid_file"
        fi
    done
    echo "Done. (Relay not stopped — use --stop-relay to stop the relay too)"
    exit 0
fi

# --- Start ---
if [[ -z "$AGENT" ]]; then
    echo "ERROR: --agent is required"
    exit 1
fi

AGENT_UPPER="${AGENT^^//-/_}"
AGENT_KEY="BUZZ_${AGENT_UPPER}_KEY"
AGENT_PUB="BUZZ_${AGENT_UPPER}_PUB"
eval "AGENT_KEY_VAL=\${$AGENT_KEY:-}"
eval "AGENT_PUB_VAL=\${$AGENT_PUB:-}"

if [[ -z "$AGENT_KEY_VAL" ]]; then
    echo "ERROR: Agent key not found for $AGENT in $KEYS_FILE"
    exit 1
fi

echo "Starting fleet processes for $AGENT..."
echo "  Relay: $BUZZ_RELAY_URL"
echo "  Channel: $FLEET_COORD_CHANNEL"
echo "  Actions config: ${ACTIONS_CONFIG:-none}"

# 1. Relay (if not already running)
RELAY_PID_FILE="/tmp/buzz-relay.pid"
if [[ -f "$RELAY_PID_FILE" ]] && ps -p "$(cat "$RELAY_PID_FILE")" >/dev/null 2>&1; then
    echo "  relay: already running (PID $(cat "$RELAY_PID_FILE"))"
else
    echo "  relay: starting..."
    # Relay is typically started separately (just relay / cargo run)
    echo "  relay: SKIPPED (start manually with 'just relay' or 'buzz-relay')"
fi

# 2. Listener (setsid + nohup for survival)
LISTENER_PID_FILE="/tmp/buzz-${AGENT}-listener.pid"
if [[ -f "$LISTENER_PID_FILE" ]] && ps -p "$(cat "$LISTENER_PID_FILE")" >/dev/null 2>&1; then
    echo "  listener: already running (PID $(cat "$LISTENER_PID_FILE"))"
else
    export BUZZ_PRIVATE_KEY="$AGENT_KEY_VAL"
    setsid nohup python3 "$SCRIPT_DIR/buzz_fleet_listen.py" --agent "$AGENT" \
        >"/tmp/buzz-${AGENT}-listener.log" 2>&1 &
    LISTENER_PID=$!
    echo "$LISTENER_PID" > "$LISTENER_PID_FILE"
    echo "  listener: started (PID $LISTENER_PID)"
fi

# 3. Responder (setsid + nohup)
RESPONDER_PID_FILE="/tmp/buzz-${AGENT}-responder.pid"
if [[ -f "$RESPONDER_PID_FILE" ]] && ps -p "$(cat "$RESPONDER_PID_FILE")" >/dev/null 2>&1; then
    echo "  responder: already running (PID $(cat "$RESPONDER_PID_FILE"))"
else
    RESPONDER_ARGS="--agent $AGENT"
    if [[ -n "$ACTIONS_CONFIG" ]]; then
        RESPONDER_ARGS="$RESPONDER_ARGS --actions-config $ACTIONS_CONFIG"
    fi
    setsid nohup python3 "$SCRIPT_DIR/buzz_autonomous_responder.py" $RESPONDER_ARGS \
        >"/tmp/buzz-${AGENT}-responder.log" 2>&1 &
    RESPONDER_PID=$!
    echo "$RESPONDER_PID" > "$RESPONDER_PID_FILE"
    echo "  responder: started (PID $RESPONDER_PID)"
fi

# 4. EKS bridge (optional)
if [[ "$WITH_EKS" -eq 1 ]]; then
    EKS_PID_FILE="/tmp/buzz-eks-bridge.pid"
    if [[ -f "$EKS_PID_FILE" ]] && ps -p "$(cat "$EKS_PID_FILE")" >/dev/null 2>&1; then
        echo "  eks_bridge: already running (PID $(cat "$EKS_PID_FILE"))"
    elif [[ -f "$SCRIPT_DIR/buzz_eks_bridge.py" ]]; then
        export BUZZ_PRIVATE_KEY="$BUZZ_COORDINATOR_KEY"
        setsid nohup python3 "$SCRIPT_DIR/buzz_eks_bridge.py" \
            --channel "$FLEET_COORD_CHANNEL" --poll-interval 10 \
            >"/tmp/buzz-eks-bridge.log" 2>&1 &
        EKS_PID=$!
        echo "$EKS_PID" > "$EKS_PID_FILE"
        echo "  eks_bridge: started (PID $EKS_PID)"
    else
        echo "  eks_bridge: SKIPPED (buzz_eks_bridge.py not found — this is a Doc2DB-specific script)"
    fi
else
    echo "  eks_bridge: SKIPPED (use --with-eks-bridge or --all)"
fi

# 5. Cloud bridge (optional)
if [[ "$WITH_CLOUD" -eq 1 ]]; then
    CLOUD_PID_FILE="/tmp/buzz-cloud-bridge.pid"
    if [[ -f "$CLOUD_PID_FILE" ]] && ps -p "$(cat "$CLOUD_PID_FILE")" >/dev/null 2>&1; then
        echo "  cloud_bridge: already running (PID $(cat "$CLOUD_PID_FILE"))"
    else
        export BUZZ_PRIVATE_KEY="$BUZZ_COORDINATOR_KEY"
        setsid nohup python3 "$SCRIPT_DIR/buzz_cloud_bridge.py" \
            --channel "$CLOUD_DISPATCH_CHANNEL" --poll-interval 15 \
            >"/tmp/buzz-cloud-bridge.log" 2>&1 &
        CLOUD_PID=$!
        echo "$CLOUD_PID" > "$CLOUD_PID_FILE"
        echo "  cloud_bridge: started (PID $CLOUD_PID)"
    fi
else
    echo "  cloud_bridge: SKIPPED (use --with-cloud-bridge or --all)"
fi

echo ""
echo "All processes started with setsid+nohup (survive shell cleanup)."
echo "Check status: ./scripts/buzz_fleet_start.sh --agent $AGENT --status"
echo "Stop all:     ./scripts/buzz_fleet_start.sh --agent $AGENT --stop"
