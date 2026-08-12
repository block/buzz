#!/usr/bin/env bash
# Chunk 11 — the verification suite.
#
# SOP Step 17's checklist plus the runnable subset of hardening-spec.md Part D.
# Three rules, all from Part D:
#   1. It must be runnable by a reviewer — one command, no setup.
#   2. It must FAIL, not warn. Every assertion exits non-zero on failure.
#   3. The network assertions must run FROM OFF-HOST, because on-host checks lie
#      about container reachability: `ss -tlnp` shows Docker's proxy listening and
#      tells you nothing about what is reachable.
#
# That third rule is why this script lives on the host and reaches into the
# target over ssh, rather than being an Ansible role that runs on the target. It
# is also why the §B2 carve-out assertions live HERE and not in an Ansible role:
# the VirtualBox NAT forward is host-side state that the guest cannot see, and
# "unreachable from another machine" is by definition not answerable from the
# target.
#
# TWO TOPOLOGIES, DETECTED NOT ASSUMED. buzz_tls_mode selects between them and this
# script derives everything from the deployed target rather than hardcoding one:
#
#   none (dev default)  no Caddy. Relay published on 127.0.0.1:3000 in the guest,
#                       reached through the loopback NAT forward. Compose set is
#                       compose.yml + compose.cohort.yml. Probes are http:// on 3000.
#   internal / acme     Caddy terminates TLS. Compose set adds compose.caddy.yml,
#                       whose `ports: !reset []` closes the relay's 3000. Probes are
#                       https:// on the host-side 8443 forward (guest 443).
#
# Passing the wrong `-f` set reports on a different stack, so it is derived, never
# typed. Everything is overridable from the environment for a target this script
# cannot introspect.
#
# Usage:
#   ./deploy verify                    # dev VM
#   TARGET=vps ./scripts/verify.sh     # a host from ansible/inventory/hosts.local.yml
#   TLS_MODE=internal ./scripts/verify.sh          # force a topology
#   COMPOSE_FILES="-f a.yml -f b.yml" ./scripts/verify.sh
set -uo pipefail

TARGET="${TARGET:-dev}"
SSH_PORT="${SSH_PORT:-2222}"
SSH_HOST="${SSH_HOST:-127.0.0.1}"
SSH_USER="${SSH_USER:-dev}"
DOMAIN="${DOMAIN:-buzz-vm.test}"
CD=/opt/buzz/compose

# The VirtualBox VM name, for the §B2 carve-out assertion on the NAT forward. Matches
# the default in virtual-box/build-vps-clone.sh and the `deploy` entrypoint.
VM="${VM:-buzz-dev}"

# Topology inputs. All may be preset; anything left empty is derived after the target
# answers, in "Topology" below. Kept empty rather than defaulted here because the
# right default depends on a value only the target knows.
TLS_MODE="${TLS_MODE:-}"
COMPOSE_FILES="${COMPOSE_FILES:-}"
AUTHORITY="${AUTHORITY:-}"
ADMIN_AUTHORITY="${ADMIN_AUTHORITY:-}"
SCHEME="${SCHEME:-}"
# Host-side probe port. Renamed from HTTPS_PORT, which is wrong for the plaintext
# path; the old name is still honoured so existing invocations keep working.
PROBE_PORT="${PROBE_PORT:-${HTTPS_PORT:-}}"

PASS=0; FAIL=0; SKIP=0
declare -a FAILED=()

vm() {
	ssh -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
		-o ConnectTimeout=6 -p "$SSH_PORT" "$SSH_USER@$SSH_HOST" "$@" 2>/dev/null
}
# Set once the topology is known. A bare `docker compose` would miss the overrides
# and report on a different stack.
DC=""

ok()   { PASS=$((PASS+1)); printf '  \033[32mPASS\033[0m  %s\n' "$1"; }
bad()  { FAIL=$((FAIL+1)); FAILED+=("$1"); printf '  \033[31mFAIL\033[0m  %s\n' "$1"; [ -n "${2:-}" ] && printf '        %s\n' "$2"; }
skip() { SKIP=$((SKIP+1)); printf '  \033[33mSKIP\033[0m  %s — %s\n' "$1" "${2:-}"; }
head_() { printf '\n\033[1m%s\033[0m\n' "$1"; }

# assert <name> <expected> <actual>
assert_eq() {
	if [ "$2" = "$3" ]; then ok "$1"; else bad "$1" "expected '$2', got '$3'"; fi
}
# assert_has <name> <needle> <haystack>
assert_has() {
	case "$3" in *"$2"*) ok "$1" ;; *) bad "$1" "expected to contain '$2', got: ${3:0:200}" ;; esac
}

printf '\033[1mBuzz deployment verification — target: %s\033[0m\n' "$TARGET"

# ---------------------------------------------------------------- reachability
head_ "Access"
if vm true; then
	ok "[HOST] ssh as $SSH_USER succeeds"
else
	bad "[HOST] ssh as $SSH_USER succeeds" "cannot reach $SSH_USER@$SSH_HOST:$SSH_PORT — nothing else can be checked"
	printf '\n\033[31mAborting: target unreachable.\033[0m\n'
	exit 1
fi

# SOP Step 17 item 9. After chunk 10 this must be REFUSED; before it, root works
# and that is the deliberately-insecure starting state, not a failure.
root_ssh=$(ssh -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
	-o ConnectTimeout=6 -p "$SSH_PORT" "root@$SSH_HOST" true 2>&1; echo "rc=$?")
hardened=$(vm 'test -f /etc/ssh/sshd_config.d/00-hardening.conf && echo yes || echo no')
if [ "$hardened" = yes ]; then
	case "$root_ssh" in
		*"rc=0"*) bad "[HOST] root ssh refused" "root login still works despite 00-hardening.conf being present — see hardening-spec.md §C2's ordering trap" ;;
		*) ok "[HOST] root ssh refused" ;;
	esac
else
	skip "[HOST] root ssh refused" "chunk 10 has not run (no 00-hardening.conf)"
fi

# ---------------------------------------------------------------- topology
# Which of the two topologies is deployed, read off the target rather than assumed.
#
# RELAY_URL is the authority here, and not by preference: it is the single control
# over which community exists and therefore which Host headers the relay accepts
# (AGENTS.md, runbooks/relay-build-list.md). Its scheme is what the relay was
# actually started with, so `wss://` means something terminates TLS in front and
# `ws://` means nothing does. BUZZ_COMPOSE_TLS is a hint for upstream's run.sh and
# can disagree with reality; the scheme cannot.
head_ "Topology"
env_present=no
vm "test -f $CD/.env" && env_present=yes

relay_url=""
if [ "$env_present" = yes ]; then
	relay_url=$(vm "sudo sed -n 's/^RELAY_URL=//p' $CD/.env" | tr -d '[:space:]')
fi

if [ -z "$TLS_MODE" ]; then
	case "$relay_url" in
		wss://*) TLS_MODE=internal ;;
		ws://*)  TLS_MODE=none ;;
		*)
			# No .env, or a RELAY_URL we cannot parse. Fall back on the target's own
			# default: `dev` is plaintext (group_vars/dev.yml), anything else is
			# production and production is never `none` (group_vars/all.yml).
			if [ "$TARGET" = dev ]; then TLS_MODE=none; else TLS_MODE=acme; fi
			;;
	esac
fi

if [ "$TLS_MODE" = none ]; then
	: "${COMPOSE_FILES:=-f compose.yml -f compose.cohort.yml}"
	: "${SCHEME:=http}"
	# Host and guest ports are identical on this path: macOS permits a non-root bind
	# on 3000, so the `relay` NAT forward translates nothing.
	: "${PROBE_PORT:=3000}"
	: "${AUTHORITY:=$DOMAIN:3000}"
	: "${ADMIN_AUTHORITY:=admin.$DOMAIN:3000}"
else
	: "${COMPOSE_FILES:=-f compose.yml -f compose.caddy.yml -f compose.cohort.yml}"
	: "${SCHEME:=https}"
	# Host-side 8443 -> guest 443. macOS forbids a non-root bind below 1024 and
	# VirtualBox's NAT forwarder runs as the invoking user, so the host side has to
	# be high. The guest side is the real 443 and matches production byte for byte.
	: "${PROBE_PORT:=8443}"
	: "${AUTHORITY:=$DOMAIN:8443}"
	: "${ADMIN_AUTHORITY:=admin.$DOMAIN:8443}"
fi

DC="cd $CD && sudo docker compose --env-file .env $COMPOSE_FILES"

printf '  %-8s %s\n' "mode" "$TLS_MODE${relay_url:+  (RELAY_URL=$relay_url)}"
printf '  %-8s %s\n' "probe" "$SCHEME://$DOMAIN:$PROBE_PORT/  Host: $AUTHORITY"
printf '  %-8s %s\n' "compose" "$COMPOSE_FILES"

# The one thing this section can get wrong in a way that matters: a plaintext
# topology on a target that is not the loopback-only dev VM. Ansible's
# roles/tls_mode_guard refuses that before any host change; this is the same
# invariant, checked against whatever is actually deployed.
if [ "$TLS_MODE" = none ] && [ "$TARGET" != dev ]; then
	bad "[HOST] plaintext is confined to the dev VM" \
		"TARGET=$TARGET is deployed with buzz_tls_mode: none. hardening-spec.md §B2: 'buzz_tls_mode: none is prohibited on every VPS, cloud host, bridged VM, host-only VM, or publicly reachable target.'"
else
	ok "[HOST] topology permitted for this target ($TLS_MODE on $TARGET)"
fi

# ---------------------------------------------------------------- the machine
head_ "Machine parity (CPU, RAM, swap — NOT disk; see SOP Step 2.5)"
assert_eq "[VM] architecture is x86_64" "x86_64" "$(vm 'uname -m')"
mem_total=$(vm "free -m | awk '/^Mem:/{print \$2}'")
if [ "${mem_total:-0}" -ge 1900 ] && [ "${mem_total:-0}" -le 2100 ]; then
	ok "[VM] memory ~1.9 GiB (${mem_total} MB)"
else
	bad "[VM] memory ~1.9 GiB" "got ${mem_total:-unknown} MB — parity with the VPS is the reason this VM exists"
fi
swap_total=$(vm "free -m | awk '/^Swap:/{print \$2}'")
assert_eq "[VM] swap is 496 MB" "495" "${swap_total:-}"

swap_used=$(vm "free -m | awk '/^Swap:/{print \$3}'")
if [ "${swap_used:-1}" -eq 0 ]; then
	ok "[VM] swap unused at idle"
else
	bad "[VM] swap unused at idle" "${swap_used} MB in use. Do NOT raise the VM's RAM — that hides the finding (SOP Step 15). Compare per-container usage against SOP Step 8.3."
fi

# ---------------------------------------------------------------- the stack
head_ "Stack"
if [ "$env_present" != yes ]; then
	skip "[VM] stack assertions" "no $CD/.env — chunks 05-07 have not run"
else
	ps_json=$(vm "$DC ps -a --format json")
	# --format json emits one object per line, not an array.
	unhealthy=$(printf '%s\n' "$ps_json" | python3 -c '
import sys, json
problems = []
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    c = json.loads(line)
    svc = c.get("Service", "?")
    if svc == "minio-init":
        # Supposed to be Exited: it creates the bucket, sets it non-public, stops.
        if c.get("ExitCode", 1) != 0:
            problems.append(svc + " exit=" + str(c.get("ExitCode")))
    elif not c.get("Health"):
        # Empty Health means the service declares no healthcheck, NOT that it is
        # unhealthy. Caddy is the case here: compose.caddy.yml defines none, so
        # `docker compose ps` reports Health="" while `up --wait` still prints
        # "Healthy". Requiring Health=="healthy" for every service therefore
        # fails on a perfectly good stack. For these, running is the bar.
        if c.get("State") != "running":
            problems.append(svc + "=" + str(c.get("State")) + " (no healthcheck)")
    elif c.get("Health") != "healthy":
        problems.append(svc + "=" + str(c.get("State")) + "/" + str(c.get("Health")))
print(",".join(problems))
' 2>/dev/null)
	if [ -z "$unhealthy" ]; then
		ok "[VM] every service healthy, minio-init exited 0"
	else
		bad "[VM] every service healthy" "$unhealthy"
	fi

	# hardening-spec.md §B2 required control 4, mode-aware.
	#
	#   TLS modes  Caddy legitimately publishes 80 and 443 on a wildcard address —
	#              that is the reverse proxy doing its job, and §B2's own nmap list
	#              deliberately omits both. Nothing else may publish at all: the relay
	#              is closed by compose.caddy.yml's `ports: !reset []`.
	#   none       Only the relay's 3000, and ONLY on loopback. 3000 is NOT simply
	#              added to the allowlist — that would let `0.0.0.0:3000` pass, which
	#              is the exact exposure §B2 is about. The bind address is checked
	#              alongside the port number, and an absent address counts as a
	#              wildcard.
	if [ "$TLS_MODE" = none ]; then
		allowed_ports="3000"; loopback_ports="3000"
	else
		allowed_ports="80,443"; loopback_ports=""
	fi
	published=$(vm "$DC ps --format json" | ALLOWED="$allowed_ports" LOOPBACK="$loopback_ports" python3 -c '
import os, sys, json
allowed  = {int(p) for p in os.environ["ALLOWED"].split(",") if p}
loopback = {int(p) for p in os.environ["LOOPBACK"].split(",") if p}
LOOPBACK_ADDRS = {"127.0.0.1", "::1", "[::1]"}
out = []
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    c = json.loads(line)
    svc = str(c.get("Service"))
    for p in (c.get("Publishers") or []):
        port = p.get("PublishedPort") or 0
        if not port:
            # Exposed but not published: not a finding.
            continue
        if port not in allowed:
            out.append(svc + ":" + str(port) + " (port not declared by this topology)")
        elif port in loopback:
            # An empty or missing URL means Docker bound every interface. Fail
            # closed: nothing here proves the binding.
            url = p.get("URL") or ""
            if url not in LOOPBACK_ADDRS:
                out.append(svc + ":" + str(port) + " bound to " + (url or "(wildcard/unset)"))
print(",".join(sorted(set(out))))
' 2>/dev/null)
	if [ "$TLS_MODE" = none ]; then
		label="[VM] only the relay's 3000 published, loopback-bound"
		hint="compose.cohort.yml's 'ports: !override [\"127.0.0.1:3000:3000\"]' is the control (§B2 control 2); ufw cannot close a Docker-published port because Docker's chain is evaluated first."
	else
		label="[VM] only Caddy's 80/443 published; relay's 3000 closed"
		hint="compose.caddy.yml's 'ports: !reset []' is the control (§B2); ufw cannot close a Docker-published port."
	fi
	if [ -z "$published" ]; then
		ok "$label"
	else
		bad "$label" "unexpected published ports: $published. $hint"
	fi

	community=$(vm "$DC exec -T postgres psql -U buzz -d buzz -tAc 'select host from communities;'" | tr -d '[:space:]')
	assert_eq "[VM] community bound to $AUTHORITY" "$AUTHORITY" "$community"
fi

# ---------------------------------------------------------------- the tests that can fail
head_ "Usability (the checks that a broken relay actually fails)"
# SOP Step 8.2: /health, /_liveness and NIP-11 return 200 even when the address is
# completely wrong, so a relay refusing every client passes all three. Only the
# WebSocket upgrade proves the community binding.
#
# --http1.1 IS LOAD-BEARING, AND ONLY OVER TLS. There curl negotiates HTTP/2 via
# ALPN, and HTTP/2 removed the Connection/Upgrade mechanism — without this flag the
# relay sees a plain GET, answers 200, and this assertion passes while proving
# nothing. Verified 2026-08-12: same request is 101 with the flag and 200 without it.
# On the plaintext path curl speaks HTTP/1.1 anyway, so the flag is harmless there
# and is kept unconditionally rather than made to appear and disappear with the mode.
ws=$(curl -sk -i -N --http1.1 --max-time 8 \
	--resolve "$DOMAIN:$PROBE_PORT:127.0.0.1" \
	-H "Connection: Upgrade" -H "Upgrade: websocket" \
	-H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: AAAAAAAAAAAAAAAAAAAAAA==" \
	"$SCHEME://$DOMAIN:$PROBE_PORT/" 2>/dev/null | head -1)
assert_has "[HOST] WebSocket upgrade accepted (101)" "101" "${ws:-no response}"

# Proves the negotiation trap is still real rather than assumed. Only meaningful
# under TLS: h2c needs --http2-prior-knowledge, so plaintext never negotiates it and
# there is no trap to demonstrate.
if [ "$TLS_MODE" = none ]; then
	skip "[HOST] HTTP/2 offered" "plaintext topology — curl speaks HTTP/1.1 and the ALPN trap does not arise"
else
	h2=$(curl -sk -o /dev/null -w '%{http_version}' --max-time 8 \
		--resolve "$DOMAIN:$PROBE_PORT:127.0.0.1" "$SCHEME://$DOMAIN:$PROBE_PORT/" 2>/dev/null)
	if [ "$h2" = "2" ]; then
		ok "[HOST] HTTP/2 is offered (so --http1.1 above is required, not cosmetic)"
	else
		skip "[HOST] HTTP/2 offered" "negotiated HTTP/$h2 — the --http1.1 flag is harmless either way"
	fi
fi

# An unknown name must be refused, and WHICH LAYER refuses it differs by topology —
# so the two branches assert different things and are not interchangeable.
if [ "$TLS_MODE" = none ]; then
	# No proxy: the relay's own community gate is directly observable from the host,
	# which is the one thing the plaintext path makes easier to prove. A response
	# body naming the community gate is the pass; a served page would mean the relay
	# is answering for a host it was never seeded with.
	unknown=$(curl -s --max-time 8 \
		--resolve "wrong.example.invalid:$PROBE_PORT:127.0.0.1" \
		"$SCHEME://wrong.example.invalid:$PROBE_PORT/" 2>/dev/null)
	assert_has "[HOST] relay refuses an unknown Host (community gate)" \
		"no community is configured" "${unknown:-empty}"
else
	# Caddy must refuse an unknown SNI outright rather than proxying it. A 200 here
	# would mean a wildcard site block, which hands §B1's admin API to anyone.
	sni=$(curl -sk -o /dev/null -w '%{http_code}' --max-time 8 \
		--resolve "wrong.example.invalid:$PROBE_PORT:127.0.0.1" \
		"https://wrong.example.invalid:$PROBE_PORT/" 2>/dev/null)
	if [ "$sni" = "000" ] || [ "$sni" = "404" ] || [ "$sni" = "421" ]; then
		ok "[HOST] Caddy refuses an unknown SNI (no wildcard site block)"
	else
		bad "[HOST] Caddy refuses an unknown SNI" "got $sni — a wildcard site block would expose the admin API (§B10)"
	fi
fi

# The relay's own community gate, from inside the Docker network. Asserted in BOTH
# topologies: under TLS it is the only place it is observable (Caddy rejects an
# unknown SNI at the handshake first), and on plaintext it proves the same gate one
# layer deeper than the host-side check above.
if [ "$env_present" = yes ]; then
	refusal=$(vm "sudo docker run --rm --network buzz-prod_buzz-net curlimages/curl:latest -s --max-time 6 -H 'Host: wrong.example.invalid' -H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: AAAAAAAAAAAAAAAAAAAAAA==' http://relay:3000/")
	assert_has "[VM] relay refuses an unmapped Host" "no community is configured" "${refusal:-empty}"
fi

# ---------------------------------------------------------------- surfaces
head_ "Relay-served surfaces"
# No explicit Host header: curl derives it from the URL, and the URL carries the port,
# so the header is `<name>:<port>` — byte-identical to the seeded community authority.
# normalize_host strips only a trailing :443 or :80, so dev's :3000 and :8443 stay part
# of the community host and must be present here (crates/buzz-core/src/tenant.rs:121).
probe() { curl -sk -o /dev/null -w '%{http_code}' --max-time 8 --resolve "$1:$PROBE_PORT:127.0.0.1" ${3:+-H "Accept: $3"} "$SCHEME://$1:$PROBE_PORT$2" 2>/dev/null; }
assert_eq "[HOST] web bundle at /" "200" "$(probe "$DOMAIN" / 'text/html')"
assert_eq "[HOST] NIP-11 document at /" "200" "$(probe "$DOMAIN" / 'application/nostr+json')"
assert_eq "[HOST] admin API refused on the ordinary host" "403" "$(probe "$DOMAIN" /api/admin/v1/reports)"

admin_sni="${ADMIN_AUTHORITY%%:*}"
# Read the value rather than counting a regex match. An earlier version used
# `grep -c '^BUZZ_ADMIN_HOST=.\+'`, whose backslash did not survive quoting
# through ssh — it returned 0 on a host where the variable WAS set, and the
# script then reported "admin API surface absent" as a PASS. A false pass on a
# security assertion is worse than a failure, so this now tests the value.
admin_value=$(vm "sudo sed -n 's/^BUZZ_ADMIN_HOST=//p' $CD/.env" | tr -d '[:space:]')
if [ -n "$admin_value" ]; then
	assert_eq "[HOST] admin dashboard at /reports" "200" "$(probe "$admin_sni" /reports 'text/html')"
	# NOTE: this 200 is UNAUTHENTICATED. hardening-spec.md §B1 — the admin API's
	# only credential is a matching Host header; there is no token, no Nostr auth
	# and no membership check. Correct on a loopback VM, an information
	# disclosure on a public host. Production leaves BUZZ_ADMIN_HOST unset so the
	# relay never mounts the router at all, and this block skips.
	assert_eq "[HOST] admin API on the admin host (unauthenticated by design in dev)" "200" "$(probe "$admin_sni" /api/admin/v1/reports)"
else
	ok "[HOST] admin API surface absent (BUZZ_ADMIN_HOST unset — the production posture, §B1)"
fi

# ---------------------------------------------------------------- §B2 carve-out
# hardening-spec.md §B2's approved development exception, and the only place in the
# tree where its two host-side conditions can be checked at all.
#
#   "Sole development exception: the dev-vm VirtualBox NAT target may use
#    buzz_tls_mode: none and publish guest port 3000 only when the corresponding host
#    NAT forward is bound exactly to 127.0.0.1:3000. Verification must assert the
#    loopback-bound forward and must demonstrate that port 3000 is unreachable from
#    another machine. If either property cannot be established, deployment fails
#    closed."
#
# WHY HERE AND NOWHERE ELSE. The NAT forward is host-side state — Ansible is inside
# the guest and cannot see it, and `VBoxManage` does not exist there. "Unreachable
# from another machine" is likewise not answerable on the target: `ss -tlnp` shows
# Docker's proxy listening and says nothing about reachability, which is the same
# reason §B2's own verification calls for nmap from off-host.
#
# The whole section is skipped under a TLS mode. There the exception does not apply:
# compose.caddy.yml unpublishes 3000 outright and §B2 control 1 is back in force.
head_ "Plaintext carve-out (hardening-spec.md §B2 — dev only)"
if [ "$TLS_MODE" != none ]; then
	skip "[HOST] §B2 carve-out assertions" "buzz_tls_mode is '$TLS_MODE' — the relay publishes no port, so the exception does not apply"
elif ! command -v VBoxManage >/dev/null 2>&1; then
	# Fails, not skips, when the target IS the dev VM: the exception is conditional on
	# a property we then cannot establish, and §B2 says that fails closed.
	if [ "$TARGET" = dev ]; then
		bad "[HOST] NAT forward for 3000 is bound to 127.0.0.1" \
			"VBoxManage is not on PATH, so the loopback-bound forward cannot be established. §B2: 'If either property cannot be established, deployment fails closed.'"
	else
		skip "[HOST] NAT forward for 3000 is bound to 127.0.0.1" "VBoxManage not present and TARGET=$TARGET is not the VirtualBox dev VM"
	fi
else
	# Forwarding(N)="name,proto,hostip,hostport,guestip,guestport" — six fields, and
	# an EMPTY hostip field is what "bind every interface" looks like here, so the
	# check is membership in a loopback set rather than "not 0.0.0.0".
	#
	# Every rule reaching guest 3000 is examined, not just the one named `relay`: a
	# second forward added later under any name reopens the same port, and the
	# exception is a property of the port, not of a rule's name.
	fwd=$(VBoxManage showvminfo "$VM" --machinereadable 2>/dev/null | grep -i '^Forwarding' || true)
	if [ -z "$fwd" ]; then
		bad "[HOST] NAT forward for 3000 is bound to 127.0.0.1" \
			"VM '$VM' reports no NAT forwards at all (is VM=$VM the right name?). The forward cannot be established, so this fails closed."
	else
		offenders=$(printf '%s\n' "$fwd" | sed -e 's/^[^=]*="//' -e 's/"$//' | awk -F, '
			$6 == "3000" {
				if ($3 != "127.0.0.1" && $3 != "::1") { print $1 "@" ($3 == "" ? "(all interfaces)" : $3) ":" $4 }
				else if ($4 != "3000")                { print $1 "@" $3 ":" $4 " (host port is not 3000)" }
			}')
		has_rule=$(printf '%s\n' "$fwd" | sed -e 's/^[^=]*="//' -e 's/"$//' | awk -F, '$6 == "3000" && $3 == "127.0.0.1" && $4 == "3000"' | wc -l | tr -d ' ')
		if [ -n "$offenders" ]; then
			bad "[HOST] NAT forward for 3000 is bound to 127.0.0.1" \
				"non-loopback forward(s) to guest 3000: $(printf '%s' "$offenders" | tr '\n' ' '). §B2 permits plaintext only while the forward is bound exactly to 127.0.0.1:3000."
		elif [ "${has_rule:-0}" -lt 1 ]; then
			bad "[HOST] NAT forward for 3000 is bound to 127.0.0.1" \
				"no forward reaches guest port 3000 at all, so the relay is unreachable from this host. Add it without a rebuild: VBoxManage controlvm $VM natpf1 \"relay,tcp,127.0.0.1,3000,,3000\"."
		else
			ok "[HOST] NAT forward 127.0.0.1:3000 -> guest 3000, loopback-bound"
		fi
	fi

	# The off-host half. A true demonstration is an nmap from a second machine, which
	# this script cannot summon — but the property it would measure is decided by the
	# bind address of the host-side listener, and that IS testable from here: dial the
	# host's own routable address. A forward bound to 127.0.0.1 refuses; one bound to
	# 0.0.0.0 answers, and would answer the same way to anyone on the network.
	#
	# Reported honestly as what it is. It cannot see a firewall between this machine
	# and another, so it is evidence for the exception's second condition, not the
	# full nmap the spec describes — which stays on the unasserted list at the end.
	lan_ip=""
	for iface in en0 en1 en2; do
		lan_ip=$(ipconfig getifaddr "$iface" 2>/dev/null) && [ -n "$lan_ip" ] && break
	done
	if [ -z "$lan_ip" ]; then
		skip "[HOST] port 3000 unreachable on a routable address" "no routable IPv4 address found (ipconfig getifaddr en0/en1/en2) — run nmap from a second machine to establish this"
	else
		reach=$(curl -s -o /dev/null -w '%{http_code}' --max-time 4 "http://$lan_ip:3000/" 2>/dev/null)
		if [ "$reach" = "000" ]; then
			ok "[HOST] port 3000 refuses connections on $lan_ip (loopback-only listener)"
		else
			bad "[HOST] port 3000 unreachable on a routable address" \
				"http://$lan_ip:3000/ answered $reach — the relay is reachable on this machine's routable address, so it is reachable from another machine. §B2 permits the plaintext exception only while it is not."
		fi
	fi
fi

# ---------------------------------------------------------------- hardening
head_ "Hardening (dev-VM subset — NOT the full production baseline)"
if [ "$hardened" != yes ]; then
	skip "[VM] hardening assertions" "chunk 10 has not run"
else
	sshd_t=$(vm 'sudo sshd -T 2>/dev/null')
	# sshd -T prints the MERGED effective config. Checking the file instead is the
	# mistake hardening-spec.md §C2 is built around: sshd keeps the FIRST value it
	# reads, so a correct-looking file can be losing to 50-cloud-init.conf or
	# 60-cloudimg-settings.conf and change nothing.
	assert_has "[VM] sshd permitrootlogin no (effective)" "permitrootlogin no" "$sshd_t"
	assert_has "[VM] sshd passwordauthentication no (effective)" "passwordauthentication no" "$sshd_t"
	assert_has "[VM] sshd kbdinteractiveauthentication no (effective)" "kbdinteractiveauthentication no" "$sshd_t"

	parity=$(vm 'ls /etc/ssh/sshd_config.d/ 2>/dev/null | grep -c -- "-parity.conf" || true')
	assert_eq "[VM] the sshd parity file is gone" "0" "${parity:-?}"

	ufw=$(vm 'sudo ufw status verbose 2>/dev/null')
	assert_has "[VM] ufw active" "Status: active" "$ufw"
	assert_has "[VM] ufw denies incoming by default" "deny (incoming)" "$ufw"

	assert_eq "[VM] kernel.kptr_restrict = 2" "2" "$(vm 'sudo sysctl -n kernel.kptr_restrict')"
	assert_eq "[VM] fs.suid_dumpable = 0" "0" "$(vm 'sudo sysctl -n fs.suid_dumpable')"
	# Deliberately NOT asserted: net.ipv4.ip_forward=0. Docker requires forwarding
	# and turns it back on, so asserting 0 would fail forever (§C5).
	assert_eq "[VM] net.ipv4.ip_forward left enabled for Docker" "1" "$(vm 'sudo sysctl -n net.ipv4.ip_forward')"

	assert_has "[VM] dockerd config validates" "configuration OK" "$(vm 'sudo dockerd --validate 2>&1')"
	assert_eq "[VM] relay container on the journald log driver" "journald" "$(vm "sudo docker inspect buzz-prod-relay-1 --format '{{.HostConfig.LogConfig.Type}}'")"
	# Asks the kernel, because .HostConfig.SecurityOpt is empty for daemon
	# defaults and would never pass (§C10 / SOP Step 13.7).
	assert_has "[VM] relay container has NoNewPrivs set" "1" "$(vm 'sudo docker exec buzz-prod-relay-1 grep NoNewPrivs /proc/1/status')"

	assert_has "[VM] unattended-upgrades restricted to -security" "-security" "$(vm 'sudo grep -h Allowed-Origins -A3 /etc/apt/apt.conf.d/52buzz-hardening 2>/dev/null')"
	if vm 'sudo grep -qr "universe" /etc/apt/apt.conf.d/52buzz-hardening 2>/dev/null'; then
		bad "[VM] unattended-upgrades excludes universe" "universe has no guaranteed security updates — you get the reboots without the safety (§C7)"
	else
		ok "[VM] unattended-upgrades excludes universe"
	fi
fi

# ---------------------------------------------------------------- hygiene
head_ "Reproducibility and hygiene"
if [ "$env_present" = yes ]; then
	assert_eq "[VM] .env is 0600 root" "600 root" "$(vm "stat -c '%a %U' $CD/.env")"
fi
# Must run on the host: chunk 05 copies only the compose bundle, so there is no
# git repository in the VM and `git grep` there fails outright. Also do NOT test
# this as "git status is clean" — activating Hermit legitimately modifies
# bin/pnpm, so the tree is dirty for reasons unrelated to secrets.
repo_root=$(cd "$(dirname "${BASH_SOURCE[0]}")/../../.." && pwd)
if git -C "$repo_root" rev-parse --git-dir >/dev/null 2>&1; then
	# Scoped to launchpad/deploy — the tree we author — and matched on REAL key
	# shapes, for a reason worth recording: SOP Step 17 item 27 specifies
	# `git grep -nE 'sk-or-v1-|OPENROUTER_API_KEY=sk'` across the whole repo, and
	# that check FAILS on a clean checkout. It matches the SOP's own prose
	# ("It starts `sk-or-v1-`"), buzz-agent/README.md's example, and
	# desktop/src-tauri/src/commands/agent_models_tests.rs, which uses
	# "sk-or-v1-secret-key-12345" as a fixture in a test *for secret redaction*.
	# A check that cries wolf on every run gets ignored, which is worse than not
	# having it. Real OpenRouter keys are long, so require length.
	hits=$(git -C "$repo_root" grep -nE \
		'sk-or-v1-[A-Za-z0-9_-]{32,}|BUZZ_RELAY_PRIVATE_KEY=[0-9a-f]{64}|POSTGRES_PASSWORD=[0-9a-f]{24}|REDIS_PASSWORD=[0-9a-f]{24}|\$6\$[./A-Za-z0-9]{8,}' \
		-- launchpad/deploy 2>/dev/null)
	if [ -n "$hits" ]; then
		bad "[HOST] no secrets committed under launchpad/deploy" "$(printf '%s' "$hits" | head -3)"
	else
		ok "[HOST] no secrets committed under launchpad/deploy"
	fi
else
	skip "[HOST] no secrets committed" "not a git repository"
fi

# ---------------------------------------------------------------- summary
head_ "Summary"
printf '  %d passed, %d failed, %d skipped\n' "$PASS" "$FAIL" "$SKIP"
if [ "$FAIL" -gt 0 ]; then
	printf '\n\033[31mFailed assertions:\033[0m\n'
	for f in "${FAILED[@]}"; do printf '  - %s\n' "$f"; done
	printf '\nThis suite covers the DEV-VM hardening subset. A clean run does NOT mean\n'
	printf 'production-hardened — see hardening-spec.md Part D for the assertions this\n'
	printf 'script does not yet make (off-host nmap, egress default-deny, datastore\n'
	printf 'network isolation, image digest pinning, TLS grade, restore drill).\n'
	exit 1
fi
printf '\n\033[32mAll assertions passed.\033[0m\n'
printf 'Scope reminder: dev-VM subset only. hardening-spec.md Part D lists what is\n'
printf 'still unasserted — notably off-host nmap, default-deny egress, datastore\n'
printf 'network isolation (§B5), digest pinning (§B12) and a timed restore drill (§E).\n'
if [ "$TLS_MODE" = none ]; then
	printf '\nTopology reminder: this run verified the PLAINTEXT dev path (buzz_tls_mode:\n'
	printf 'none). It is hardening-spec.md §B2'"'"'s sole development exception and is\n'
	printf 'prohibited on every VPS, cloud host, bridged VM, host-only VM or publicly\n'
	printf 'reachable target. Nothing above rehearses the production TLS controls —\n'
	printf 'compose.caddy.yml'"'"'s "ports: !reset []", the Caddyfile'"'"'s explicit site blocks\n'
	printf 'and the admin vhost split are exercised only under TLS_MODE=internal. The\n'
	printf '"unreachable from another machine" check above dials this host'"'"'s own routable\n'
	printf 'address; the full off-host nmap from a second machine remains unasserted.\n'
fi
