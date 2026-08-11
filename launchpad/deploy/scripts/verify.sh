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
# target over ssh, rather than being an Ansible role that runs on the target.
#
# Usage:
#   ./deploy verify                    # dev VM
#   TARGET=vps ./scripts/verify.sh     # a host from ansible/inventory/hosts.local.yml
set -uo pipefail

TARGET="${TARGET:-dev}"
SSH_PORT="${SSH_PORT:-2222}"
SSH_HOST="${SSH_HOST:-127.0.0.1}"
SSH_USER="${SSH_USER:-dev}"
DOMAIN="${DOMAIN:-buzz-vm.test}"
AUTHORITY="${AUTHORITY:-buzz-vm.test:8443}"
ADMIN_AUTHORITY="${ADMIN_AUTHORITY:-admin.buzz-vm.test:8443}"
HTTPS_PORT="${HTTPS_PORT:-8443}"
CD=/opt/buzz/compose

PASS=0; FAIL=0; SKIP=0
declare -a FAILED=()

vm() {
	ssh -o BatchMode=yes -o StrictHostKeyChecking=no -o UserKnownHostsFile=/dev/null \
		-o ConnectTimeout=6 -p "$SSH_PORT" "$SSH_USER@$SSH_HOST" "$@" 2>/dev/null
}
# The three-file compose invocation. A bare `docker compose` here would miss
# compose.caddy.yml and compose.cohort.yml and report on a different stack.
DC="cd $CD && sudo docker compose --env-file .env -f compose.yml -f compose.caddy.yml -f compose.cohort.yml"

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
if ! vm "test -f $CD/.env"; then
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

	# hardening-spec.md §B2 required control 4.
	published=$(vm "$DC ps --format json" | python3 -c '
import sys, json
out = []
for line in sys.stdin:
    line = line.strip()
    if not line:
        continue
    c = json.loads(line)
    for p in (c.get("Publishers") or []):
        port = p.get("PublishedPort") or 0
        # Caddy legitimately publishes 80 and 443. Nothing else may publish at all —
        # the relay depends on compose.caddy.yml unsetting its ports (§B2).
        if port and port not in (80, 443):
            out.append(str(c.get("Service")) + ":" + str(port))
print(",".join(sorted(set(out))))
' 2>/dev/null)
	if [ -z "$published" ]; then
		ok "[VM] only Caddy's 80/443 published; relay's 3000 closed"
	else
		bad "[VM] only Caddy's 80/443 published" "unexpected published ports: $published. compose.caddy.yml's 'ports: !reset []' is the control (§B2); ufw cannot close a Docker-published port."
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
# --http1.1 IS LOAD-BEARING. Over TLS curl negotiates HTTP/2 via ALPN, and HTTP/2
# removed the Connection/Upgrade mechanism — without this flag the relay sees a
# plain GET, answers 200, and this assertion passes while proving nothing.
# Verified 2026-08-12: same request is 101 with the flag and 200 without it.
ws=$(curl -sk -i -N --http1.1 --max-time 8 \
	--resolve "$DOMAIN:$HTTPS_PORT:127.0.0.1" \
	-H "Connection: Upgrade" -H "Upgrade: websocket" \
	-H "Sec-WebSocket-Version: 13" -H "Sec-WebSocket-Key: AAAAAAAAAAAAAAAAAAAAAA==" \
	"https://$DOMAIN:$HTTPS_PORT/" 2>/dev/null | head -1)
assert_has "[HOST] WebSocket upgrade accepted (101)" "101" "${ws:-no response}"

# Proves the negotiation trap is still real rather than assumed.
h2=$(curl -sk -o /dev/null -w '%{http_version}' --max-time 8 \
	--resolve "$DOMAIN:$HTTPS_PORT:127.0.0.1" "https://$DOMAIN:$HTTPS_PORT/" 2>/dev/null)
if [ "$h2" = "2" ]; then
	ok "[HOST] HTTP/2 is offered (so --http1.1 above is required, not cosmetic)"
else
	skip "[HOST] HTTP/2 offered" "negotiated HTTP/$h2 — the --http1.1 flag is harmless either way"
fi

# Caddy must refuse an unknown SNI outright rather than proxying it. A 200 here
# would mean a wildcard site block, which hands §B1's admin API to anyone.
sni=$(curl -sk -o /dev/null -w '%{http_code}' --max-time 8 \
	--resolve "wrong.example.invalid:$HTTPS_PORT:127.0.0.1" \
	"https://wrong.example.invalid:$HTTPS_PORT/" 2>/dev/null)
if [ "$sni" = "000" ] || [ "$sni" = "404" ] || [ "$sni" = "421" ]; then
	ok "[HOST] Caddy refuses an unknown SNI (no wildcard site block)"
else
	bad "[HOST] Caddy refuses an unknown SNI" "got $sni — a wildcard site block would expose the admin API (§B10)"
fi

# The relay's own community gate, only observable behind Caddy.
if vm "test -f $CD/.env"; then
	refusal=$(vm "sudo docker run --rm --network buzz-prod_buzz-net curlimages/curl:latest -s --max-time 6 -H 'Host: wrong.example.invalid' -H 'Connection: Upgrade' -H 'Upgrade: websocket' -H 'Sec-WebSocket-Version: 13' -H 'Sec-WebSocket-Key: AAAAAAAAAAAAAAAAAAAAAA==' http://relay:3000/")
	assert_has "[VM] relay refuses an unmapped Host" "no community is configured" "${refusal:-empty}"
fi

# ---------------------------------------------------------------- surfaces
head_ "Relay-served surfaces"
probe() { curl -sk -o /dev/null -w '%{http_code}' --max-time 8 --resolve "$1:$HTTPS_PORT:127.0.0.1" ${3:+-H "Accept: $3"} "https://$1:$HTTPS_PORT$2" 2>/dev/null; }
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
if vm "test -f $CD/.env"; then
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
