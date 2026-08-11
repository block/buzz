# Gap Analysis: `hardening-linux-servers.md`

**Title:** Gap analysis of the zero-trust Ubuntu hardening report
**Summary:** Review of `hardening-linux-servers.md` for use as the basis of dev + production deployment scripting. Flags four snippets that would break or weaken a host if scripted as written, identifies the core conceptual gap (SSH hardening is not zero trust), and enumerates 20 missing control families with concrete implementations.
**Tags:** `security` `zero-trust` `ubuntu` `hardening` `deployment` `review` `hipaa`
**Reviewed:** 2026-08-11 · **Source doc:** `hardening-linux-servers.md`

---

## Verdict

As a survey of 2018-era Linux hardening it's solid and well-organized. As a **specification for deployment scripting** it has three problems:

1. **Four snippets are wrong in ways that fail loudly or silently weaken the host.** One of them (`daemon.json`) prevents Docker from starting at all, and another disables the exact protection the doc recommends two sections earlier.
2. **The zero-trust framing is asserted, not implemented.** The controls are perimeter hardening with a zero-trust vocabulary layered on top. The single biggest omission: nothing removes standing credentials or standing network reachability.
3. **The highest-leverage modern Linux controls are absent entirely** — systemd sandboxing, egress filtering, workload identity, IMDSv2, TPM-backed disk unlock, log tamper-evidence.

Everything below is ordered so you can work top-down.

---

## Part 1 — Fix before anything gets scripted

### 1.1 The `daemon.json` example breaks Docker and undoes the doc's own advice

`hardening-linux-servers.md:229-236` writes this file:

```json
{
  "disable-legacy-registry": true,
  "userns-remap": "default",
  "ip-forward-no-drop": true
}
```

Two defects, both verified:

- **`disable-legacy-registry` was removed in Docker 17.12 and `dockerd` refuses to start when it is present.** The error is `the "disable-legacy-registry" configuration option has been removed`. Any host that runs this snippet loses Docker on the next daemon restart — which, since the snippet ends in `systemctl restart docker`, is immediately.
- **`ip-forward-no-drop: true` is the documented opt-out from Docker 28's default-drop of unpublished ports** — the very protection recommended at `:137` ("With Docker Engine 28+, traffic to unpublished ports is **dropped by default**"). The snippet turns it off. This is the most dangerous line in the document because it fails *silently*: containers become reachable from the LAN and nothing logs a complaint.

`userns-remap` is real and correct, but note it is incompatible with `--privileged`, breaks bind-mount ownership, and cannot be enabled on a host with existing containers/images without a data migration. It is not a drop-in flag.

**Replacement:**
```json
{
  "userns-remap": "default",
  "no-new-privileges": true,
  "icc": false,
  "live-restore": true,
  "log-driver": "journald",
  "userland-proxy": false,
  "default-ulimits": { "nofile": { "Name": "nofile", "Hard": 4096, "Soft": 1024 } }
}
```
Then `dockerd --validate` before restarting, and gate the restart on it in the script.

### 1.2 `iptables -I DOCKER-USER -j DROP` blocks return traffic

`:138` says to insert an unconditional `DROP` at the top of `DOCKER-USER` "then allow published ports." Rule order in iptables is evaluation order — an unconditional `DROP` inserted at position 1 is terminal for every packet, including established/related return traffic, and any subsequent `-A` allow rules sit below it and never match. Containers lose all networking.

**Replacement — state rule first, DROP last:**
```bash
iptables -I DOCKER-USER 1 -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -I DOCKER-USER 2 -s 10.0.0.0/8 -j ACCEPT     # your trusted CIDR
iptables -A DOCKER-USER -j DROP                        # append: last resort
```

### 1.3 `unattended-upgrades` for "universe-provided kernels"

`:60` recommends enabling unattended-upgrades "for the *universe*-provided kernels and packages." This is backwards. `universe` is community-maintained and receives **no guaranteed security updates** from Canonical; it is the one component you would not auto-patch on a regulated host. The origin you want is `main`/`restricted` via `${distro_id}:${distro_codename}-security`, plus `${distro_id}ESMApps` and `${distro_id}ESM` if you are on Ubuntu Pro.

**Replacement (`/etc/apt/apt.conf.d/50unattended-upgrades`):**
```
Unattended-Upgrade::Allowed-Origins {
        "${distro_id}:${distro_codename}-security";
        "${distro_id}ESMApps:${distro_codename}-apps-security";
        "${distro_id}ESM:${distro_codename}-infra-security";
};
Unattended-Upgrade::Automatic-Reboot "false";
Unattended-Upgrade::Automatic-Reboot-WithUsers "false";
```
Leave reboot to your orchestration, not to the timer — see §3.14.

### 1.4 `node --security` does not exist

`:115` suggests "Enable `node --security`." There is no such flag. The real control is the **Node permission model**: `--experimental-permission` on Node 20/22, promoted to `--permission` in Node 23.5+, combined with `--allow-fs-read`, `--allow-fs-write`, `--allow-child-process`, `--allow-worker`, `--allow-addons`, `--allow-net`. Anything the script writes today should target `--permission` with an explicit allowlist, and note that the permission model is process-wide and does not survive `child_process` unless separately allowed.

### 1.5 Smaller corrections

| Line | Issue | Correction |
|---|---|---|
| `:56` | "log retention 6 years for HIPAA" | The 6-year rule (§164.316(b)(2)) covers **documentation** — policies, risk analyses, designations. Log retention itself is not fixed by HIPAA; you set it by risk analysis and state law. Don't hardcode 6 years as a HIPAA requirement in the script's comments; do document why you chose whatever you chose. |
| `:134` | "Socket is default after 0.5.2 to avoid CSRF" | Garbled. The point is: the daemon listens on a unix socket by default; do not add `-H tcp://`. If you must, require `--tlsverify` with client certs. |
| `:22` | "strong Ciphers/MTU" | MTU is not an SSH security parameter. Meant `Ciphers`/`MACs`/`KexAlgorithms`. |
| `:119` | "strong Diffie-Hellman group (4096-bit)" | Irrelevant under TLS 1.3 and ECDHE. `ssl_dhparam` only affects legacy `DHE` suites you should not be offering. Drop it rather than generating a 4096-bit param file. |
| `:123`, `:240` | `X-Frame-Options: DENY` | Superseded by CSP `frame-ancestors 'none'`. Keep XFO only for legacy clients; CSP is the enforcing control. |
| `:117` | `DELETE FROM mysql.user ...` | Direct `mysql.user` manipulation is unsupported on MySQL 8. Use `DROP USER`/`RENAME USER`. |
| `:121` | HSTS `preload` | Recommend it without the caveat that preload submission is effectively irreversible on a browser-release timescale, and will break any subdomain you later need on plain HTTP. Do not set `preload` in a script by default. |
| `:20` | typo | "Identity/**Acess** Mgmt" |
| `:56`, `:123` | dead citations | `see [75] snippet`, `as [40] warns`, and a References section that names sources but links to none. In a folder meant to be agent-searchable, dangling markers are worse than no markers — an agent will try to resolve them. Either restore the URLs or strip the brackets. |

**Confirmed accurate, and worth keeping:** the Docker 28 default-drop claim (`:137`) and the "Docker Content Trust retired, use Cosign" claim (`:135`, `:163`). Add the hard date — Notary v1 / `notary.docker.io` **shuts down 8 December 2026**, with brownouts before then. Anything still using `DOCKER_CONTENT_TRUST=1` needs to be migrated this year.

---

## Part 2 — The core gap: this is perimeter hardening, not zero trust

The doc's own definition (`:34`) is right: "no actor or network is inherently trusted; every access goes through a policy decision." But the implementation keeps two forms of standing trust that zero trust exists to eliminate:

**Standing network reachability.** Every design in the doc has `sshd` listening on a public interface, hardened. Zero trust says the port should not be reachable at all. Options, roughly in order of operational cost:

- **AWS SSM Session Manager / GCP IAP / Azure Bastion** — no inbound port, no bastion host, access is an IAM decision, sessions are logged to CloudWatch/S3 by default. Cheapest path to "no SSH on the internet."
- **WireGuard mesh with device identity** — Tailscale/Netbird with SSO + device posture, ACLs per-service. Good fit if you need more than SSH (DB admin, internal dashboards).
- **Teleport / Boundary** — full audit trail, session recording, per-session RBAC. Heaviest, best evidence story for an auditor.

Whichever you pick, the firewall rule becomes `ufw deny 22` and the SOP never mentions a public IP again.

**Standing credentials.** `authorized_keys` is a bearer credential that is valid until someone remembers to remove it. That is the opposite of continuous verification, and `:54`'s "re-authenticate on every session" cannot be built on it. Replace with an **SSH certificate authority** issuing short-lived (5–15 min) certificates:

```
# /etc/ssh/sshd_config
TrustedUserCAKeys /etc/ssh/ca.pub
AuthorizedPrincipalsFile /etc/ssh/auth_principals/%u
RevokedKeys /etc/ssh/revoked_keys
AuthorizedKeysFile none          # no standing keys, at all
```

Issue certs from `step-ca`, Vault's SSH secrets engine, or Teleport, gated on your IdP + MFA. Revocation becomes "stop issuing" instead of "find every host and edit a file" — which is also the answer to the **kill-switch question the doc never asks**: how do you revoke one compromised host's access to everything, in one action?

**Break-glass is missing and is not optional.** The moment you disable root login, remove standing keys, and put access behind an IdP, you have created a total-lockout failure mode (IdP outage, CA expiry, mesh control-plane down). You need a documented emergency path — sealed credential in a physical safe or separate break-glass vault, cloud serial console access, and **an alert that fires the instant it is used**. Script the alerting; do not script the credential.

---

## Part 3 — Missing control families

Each of these is absent from the source doc and each is directly scriptable.

### 3.1 systemd service sandboxing — the biggest single omission

Not mentioned once, and it is the highest-value-per-line hardening available on modern Ubuntu. It is more reliable than AppArmor for most services because it is declarative, per-unit, and verifiable with a score.

```ini
# /etc/systemd/system/nginx.service.d/hardening.conf
[Service]
NoNewPrivileges=yes
PrivateTmp=yes
PrivateDevices=yes
ProtectSystem=strict
ProtectHome=yes
ProtectProc=invisible
ProcSubset=pid
ProtectKernelTunables=yes
ProtectKernelModules=yes
ProtectKernelLogs=yes
ProtectControlGroups=yes
ProtectClock=yes
ProtectHostname=yes
RestrictAddressFamilies=AF_UNIX AF_INET AF_INET6
RestrictNamespaces=yes
RestrictSUIDSGID=yes
RestrictRealtime=yes
LockPersonality=yes
MemoryDenyWriteExecute=yes
SystemCallFilter=@system-service
SystemCallFilter=~@privileged @resources @obsolete
SystemCallArchitectures=native
CapabilityBoundingSet=CAP_NET_BIND_SERVICE
AmbientCapabilities=CAP_NET_BIND_SERVICE
ReadWritePaths=/var/log/nginx /var/cache/nginx
IPAddressDeny=any
IPAddressAllow=10.0.0.0/8 127.0.0.0/8
```

Two things to build into the pipeline: `systemd-analyze security <unit>` emits a numeric exposure score — **assert a maximum in CI** (e.g. fail if any unit scores worse than 3.0). And note `IPAddressAllow`/`IPAddressDeny` is a **per-unit egress firewall**, which is how you get §3.2 for free on anything running under systemd.

### 3.2 Egress filtering — absent

The doc's firewall is ingress-only (`iptables -P INPUT DROP`, `ufw default deny incoming`). `OUTPUT` is left wide open. Every post-exploitation step — C2 beacon, tool download, PHI exfiltration, crypto-miner pool connection — is *egress*. For a zero-trust build this is not optional:

```bash
iptables -P OUTPUT DROP
iptables -A OUTPUT -m conntrack --ctstate ESTABLISHED,RELATED -j ACCEPT
iptables -A OUTPUT -o lo -j ACCEPT
iptables -A OUTPUT -d 10.0.1.10 -p tcp --dport 5432 -j ACCEPT      # db
iptables -A OUTPUT -m owner --uid-owner apt -p tcp --dport 443 -j ACCEPT
iptables -A OUTPUT -j LOG --log-prefix "EGRESS-DENY "
```

Because allowlisting by IP breaks against CDN-hosted repos, pair it with an **explicit egress proxy** (Squid or Envoy with SNI allowlisting) and set `http_proxy`/`https_proxy` for `apt` and the app. Then log and alert on every proxy denial — denied egress is one of the highest-signal detections you will have.

### 3.3 DNS is not covered at all

DNS is both an exfil channel and a trust dependency. Missing: DNSSEC validation, encrypted transport, and control over what the host can resolve.

```ini
# /etc/systemd/resolved.conf
DNS=10.0.0.2#dns.internal
DNSOverTLS=yes
DNSSEC=yes
DNSStubListener=yes
Domains=~.
```
Plus: block outbound :53 to anything but your resolver (§3.2), run an RPZ/sinkhole on the resolver, and log all queries into the SIEM. NXDOMAIN bursts and high-entropy subdomain queries are how you catch DNS tunneling.

### 3.4 Cloud instance metadata — IMDSv2 not mentioned

For a "cloud-style VPS" doc this is a serious hole. The metadata service is the standard pivot from a web-app SSRF to full cloud credentials, and it is one API call to close:

```bash
aws ec2 modify-instance-metadata-options --instance-id i-xxx \
  --http-tokens required --http-put-response-hop-limit 1 --http-endpoint enabled
```
`--http-tokens required` kills IMDSv1. `--hop-limit 1` prevents a container from reaching it through the host. Set it at the **account level** as a default, and block `169.254.169.254` from every container network. Equivalents: GCP `Metadata-Flavor: Google` header requirement, Azure IMDS `Metadata: true`.

Related and also missing: **workload identity**. Nothing in the doc explains how a service authenticates to another service. Use cloud role attachment (instance profile / workload identity federation) so there are no long-lived keys on disk at all, and SPIFFE/SPIRE or `cert-manager` if you need mTLS between internal services.

### 3.5 Secret-zero and runtime secret delivery

The doc says "use Vault" in four places but never answers how the host authenticates to Vault. Specify:

- **Auth method:** Vault AWS/GCP/Azure auth (instance identity) or AppRole with a response-wrapped, single-use SecretID delivered at provision time. Not a token in userdata.
- **Delivery:** `systemd` credentials — `LoadCredential=`/`ImportCredential=` puts secrets in a per-service tmpfs at `$CREDENTIALS_DIRECTORY`, readable only by that unit. This is strictly better than the two common patterns.
- **Do not use `EnvironmentFile` or `-e` for secrets.** Environment variables are visible in `/proc/<pid>/environ`, leak into core dumps, crash reporters, `docker inspect`, and child processes. This deserves an explicit "never do this" in the SOP.
- **Never bake secrets into images.** Also: `docker build --secret` for build-time, not `ARG`, which is recorded in image history.
- **Rotation:** define a rotation interval and make rotation non-eventful. Dynamic DB credentials (Vault database engine, 1h TTL) mean a leaked credential expires on its own.

### 3.6 Disk encryption has no headless unlock story

`:62` says "enable LUKS on data drives." On a headless VPS, who enters the passphrase after a reboot? Unanswered, this control silently becomes "encryption we never actually turned on." The answer:

```bash
systemd-cryptenroll --tpm2-device=auto --tpm2-pcrs=0+7+14 /dev/sdX
```
TPM2-sealed unlock bound to measured-boot PCRs, so the disk only unlocks on an unmodified boot chain. Where there is no TPM, use **Clevis + Tang** for network-bound disk encryption. Add: measured boot (`ProtectKernelImage`, Secure Boot with your own keys if the platform allows), and `dm-verity` or IMA/EVM for a read-only integrity-measured root if you go the immutable route (§3.13).

Also missing at the data layer: **disk encryption does nothing against a compromised application** — it only protects against physical/volume-level theft. For PHI you need application-layer field encryption or tokenization for the sensitive columns, with keys in KMS and envelope encryption, so a SQL-injection read returns ciphertext.

### 3.7 sysctl list is roughly a third of what it should be

The five settings at `:64-68` and `:219-225` cover network basics and miss the entire kernel-exploit-mitigation and info-leak surface:

```conf
# kernel hardening
kernel.kptr_restrict = 2
kernel.dmesg_restrict = 1
kernel.printk = 3 3 3 3
kernel.yama.ptrace_scope = 1
kernel.unprivileged_bpf_disabled = 1
net.core.bpf_jit_harden = 2
kernel.kexec_load_disabled = 1
kernel.sysrq = 0
kernel.perf_event_paranoid = 3
kernel.randomize_va_space = 2
kernel.core_pattern = |/bin/false
dev.tty.ldisc_autoload = 0
vm.unprivileged_userfaultfd = 0
# filesystem
fs.protected_symlinks = 1
fs.protected_hardlinks = 1
fs.protected_fifos = 2
fs.protected_regular = 2
fs.suid_dumpable = 0
# network, beyond the doc's five
net.ipv4.conf.all.accept_redirects = 0
net.ipv4.conf.all.secure_redirects = 0
net.ipv4.conf.all.accept_source_route = 0
net.ipv4.conf.all.log_martians = 1
net.ipv4.conf.all.arp_ignore = 1
net.ipv4.conf.all.arp_announce = 2
net.ipv4.icmp_echo_ignore_broadcasts = 1
net.ipv4.icmp_ignore_bogus_error_responses = 1
net.ipv6.conf.all.accept_ra = 0
net.ipv6.conf.all.accept_redirects = 0
```
Note `net.ipv4.ip_forward = 0` at `:65`/`:220` **conflicts with running Docker** — Docker requires forwarding and will re-enable it. Scripting both is a guaranteed drift alarm; decide per host role and comment the exception.

Module blacklisting at `:69` mentions `dccp`/`sctp`. Extend to filesystems and buses, and use `install <mod> /bin/false` (blacklist alone does not stop explicit loads):
```
install cramfs /bin/false
install freevxfs /bin/false
install jffs2 /bin/false
install hfs /bin/false
install hfsplus /bin/false
install udf /bin/false
install usb-storage /bin/false
install firewire-core /bin/false
install thunderbolt /bin/false
install dccp /bin/false
install sctp /bin/false
install rds /bin/false
install tipc /bin/false
```

### 3.8 Availability / DoS — an objective with no controls

`:7` names availability as an objective; nothing in the doc implements it at the application edge. For a hospital portal:

```nginx
limit_req_zone $binary_remote_addr zone=perip:10m rate=10r/s;
limit_conn_zone $binary_remote_addr zone=connperip:10m;
limit_req zone=perip burst=20 nodelay;
limit_conn connperip 20;
client_body_timeout 10s;
client_header_timeout 10s;
send_timeout 10s;
client_max_body_size 10m;
client_body_buffer_size 16k;
large_client_header_buffers 4 8k;
keepalive_timeout 30s;
reset_timedout_connection on;
```
Plus: an upstream CDN/DDoS layer (Cloudflare, AWS Shield Advanced) with the origin locked to the CDN's IP ranges only — otherwise the origin IP leak defeats the whole thing; `proxy_read_timeout`/`proxy_connect_timeout` on upstreams; SYN backlog tuning (`net.ipv4.tcp_max_syn_backlog`, `somaxconn`); and per-service `MemoryMax=`/`TasksMax=`/`CPUQuota=` in systemd so one runaway service cannot take the host down.

### 3.9 Log integrity — stated as an objective, no mechanism

`:7` wants "write-once logs." An attacker with root deletes `/var/log`, so local hardening is not the control — **getting logs off the host before they can be tampered with** is.

- Ship first: `rsyslog` with RELP + TLS + disk-assisted queueing, or a Vector/Fluent Bit agent, so events leave within seconds.
- Tamper-evidence locally: journald Forward Secure Sealing — `journalctl --setup-keys --interval=1h`, then `Seal=yes` in `journald.conf`. Verify with `journalctl --verify`.
- Immutable at rest: S3 with **Object Lock in compliance mode** + a retention period, written by a role that has `PutObject` but not `DeleteObject`.
- **AIDE's database must live off-host or be signed** (`:54`, `:145` miss this) — an attacker who can modify files can regenerate the baseline.
- Trustworthy time: `chrony` with **NTS** (`server time.cloudflare.com nts`), not plain NTP. Unauthenticated NTP means an attacker can move your log timestamps.

Also missing: `auditd` is recommended (`:98`, `:143`) but with no ruleset. Do not hand-write one — start from the Neo23x0 or DISA STIG audit rules, add `-e 2` to make the config immutable until reboot, and set `space_left_action`/`disk_full_action` deliberately (the default `SUSPEND` can halt the host).

And **`sudo` I/O logging** is the concrete version of the doc's vague "SSH session recording" (`:77`):
```
Defaults use_pty
Defaults log_output
Defaults iolog_dir=/var/log/sudo-io
Defaults!/usr/bin/sudoreplay !log_output
```
With `sudo_logsrvd` to ship the I/O logs centrally.

### 3.10 Response headers and cookies are incomplete

The doc has HSTS, XFO, nosniff, and a mention of CSP. Missing the modern set:

```nginx
add_header Content-Security-Policy "default-src 'none'; script-src 'self' 'nonce-$request_id'; style-src 'self'; img-src 'self' data:; connect-src 'self'; frame-ancestors 'none'; base-uri 'none'; form-action 'self'; object-src 'none'" always;
add_header Referrer-Policy "strict-origin-when-cross-origin" always;
add_header Permissions-Policy "geolocation=(), camera=(), microphone=(), payment=()" always;
add_header Cross-Origin-Opener-Policy "same-origin" always;
add_header Cross-Origin-Embedder-Policy "require-corp" always;
add_header Cross-Origin-Resource-Policy "same-origin" always;
```
Cookies: `Secure; HttpOnly; SameSite=Lax` minimum, `__Host-` prefix for session cookies, short absolute lifetime plus idle timeout, rotate session ID on privilege change. Also missing: **CAA DNS records** to constrain who can issue for your domain, **Certificate Transparency monitoring** to detect mis-issuance, `ssl_session_tickets off` (or rotate ticket keys — static ticket keys break forward secrecy), and a deliberate decision on TLS 1.3 **0-RTT** (`ssl_early_data`) which is replay-vulnerable and should stay off for anything non-idempotent.

Forward-looking, and relevant if "government" is real: **post-quantum key exchange**. Hybrid `X25519MLKEM768` is available in current OpenSSL/nginx and is becoming the default; CNSA 2.0 sets migration deadlines. Worth a line in the TLS policy so the choice is deliberate.

### 3.11 Database layer needs concrete config

`:117` is generic. For Postgres specifically:

```
# pg_hba.conf — no trust, no md5, TLS-only, cert or SCRAM
hostssl  appdb  appuser  10.0.1.0/24  scram-sha-256
hostssl  appdb  reporting 10.0.2.0/24 cert clientcert=verify-full
local    all    all                    peer
```
Plus: `ssl_min_protocol_version = 'TLSv1.3'`, `log_connections`/`log_disconnections`/`log_statement='ddl'`, `pgaudit` for DML on PHI tables, row-level security with `FORCE ROW LEVEL SECURITY`, and revoke `CREATE` on `public` schema. Watch two superuser-to-RCE paths the doc misses: `COPY ... TO PROGRAM` and untrusted extension creation — the app role must never be superuser or hold `pg_execute_server_program`.

### 3.12 Backups: the ransomware target, treated as a chore

`:169` covers the basics but misses the properties that decide whether you survive:

- **Immutability** — object-lock / WORM, so a compromised host cannot delete its own backups.
- **Credential separation** — the backup writer role can `Put` but not `Delete` or `List`; restore uses a different, human-gated credential.
- **Separate blast radius** — different cloud account, different region, different provider for the last copy.
- **Encryption keys stored separately from backups**, with a documented recovery path for the keys themselves (a backup you cannot decrypt is not a backup).
- **Numbers, not adjectives** — define RPO and RTO, then prove them in a timed restore drill and record the measured time as evidence. "Test restores regularly" is not auditable; "restored 340 GB in 47 min on 2026-07-02, RTO target 2h" is.
- **Backup integrity verification** — checksum on write, periodic scrub, and a restore-and-query smoke test in CI against a scratch environment.

### 3.13 No immutable-infrastructure path — which is what you actually want for prod

The doc assumes long-lived, mutated hosts, then tries to secure them. Given you are writing this scripting from scratch, the stronger architecture:

1. **Golden images** built with Packer from an Ubuntu CIS/STIG base, hardening applied at build time, scanned and signed.
2. **Immutable deploy** — replace instances, never patch in place. This makes "did the hardening drift?" unanswerable-by-construction.
3. **Read-only root** at runtime (`ProtectSystem=strict`, or `/` mounted `ro` with tmpfs/overlay for the few writable paths).
4. **No SSH into production at all** — debugging happens through logs, metrics, and ephemeral break-glass (§Part 2).

If you keep mutable hosts, you need the thing the doc lacks: **continuous drift enforcement** — Ansible-pull on a systemd timer, or Chef/Puppet, alerting on every corrected deviation rather than silently fixing it.

### 3.14 Patch and reboot orchestration

`unattended-upgrades` alone leaves kernel CVEs unmitigated until a reboot nobody schedules. Add:

- **Canonical Livepatch** (Ubuntu Pro) for kernel CVEs without reboot.
- `needrestart -b` to detect services running against deleted libraries, and restart them — the classic "patched but still vulnerable" gap.
- Detect `/var/run/reboot-required` and drive reboots through the orchestrator with load-balancer drain, one instance at a time, inside a maintenance window.
- **Written patch SLAs** by severity (e.g. critical/actively-exploited within 72h, high 7d, else 30d) and a documented exception process with expiry. Auditors ask for this specifically; it does not exist in the doc.

### 3.15 Verification is missing from the pipeline

The doc audits *periodically* (`:179`, `:201`). For scripting, hardening must be **tested like code**, in CI, on every change:

- `openscap`/`oscap-ssh` against the Ubuntu CIS or DISA STIG profile — fail the build on regression.
- `goss`/`InSpec` for your own assertions ("port 22 not listening from outside", "OUTPUT policy is DROP", "no unit scores worse than 3.0").
- `lynis audit system --cronjob` for a trended score.
- `testssl.sh --severity HIGH` against the deployed endpoint.
- External `nmap -sS -p- ` from outside the VPC — the only honest test of your ingress rules.
- Container image scanning (Trivy/Grype) **with a failing threshold**, not just a report.

Store every run's output as compliance evidence (§3.19). This turns the checklist into a gate.

### 3.16 CI/CD is the highest-privilege system and is under-covered

`:159-165` covers scanning and secrets but not the runner as an attack path:

- **OIDC federation to the cloud, no long-lived deploy keys.** GitHub Actions → AWS role assumption via OIDC, with the trust policy scoped to a specific repo *and ref* (a wildcard `sub` here is a full account compromise).
- Self-hosted runners must be **ephemeral** and never used for `pull_request` from forks.
- `pull_request_target` and workflows that check out untrusted refs with secrets in scope — the standard CI escape.
- Pin actions **to commit SHAs**, not tags (tags are mutable).
- Branch protection, required review, signed commits, `CODEOWNERS` on the deployment directory.
- Lockfile integrity (`npm ci`, `--frozen-lockfile`, `cargo --locked`), dependency-confusion protection via scoped registries.
- **SBOM + provenance** — Syft for SBOM, SLSA provenance attestation, Cosign signing, and **verify at deploy time** (`cosign verify --certificate-identity-regexp`). Generating attestations nobody verifies is theater.

### 3.17 Kubernetes, if it's in scope

The doc mentions K8s network policies once and covers nothing else. If prod might be K8s, that is an entire missing chapter: Pod Security Admission at `restricted`, `seccompProfile: RuntimeDefault`, `readOnlyRootFilesystem`, `runAsNonRoot`, drop all capabilities, default-deny `NetworkPolicy` per namespace, RBAC without wildcard verbs, `automountServiceAccountToken: false`, etcd encryption at rest with a KMS provider, admission policy via Kyverno/Gatekeeper, and no cluster-admin in CI. **Worth deciding scope explicitly before scripting** — the controls diverge a lot from VM-based deploys.

### 3.18 Compliance mapping, given the stated gov/healthcare context

The doc names HIPAA and ISO 27001 but provides no control mapping, which is the artifact an auditor actually wants. Missing:

- **A control matrix**: NIST 800-53 control ID → implementation (which Ansible task / systemd directive) → evidence artifact (which scan output). Build this as a table in the repo and generate it from the code where possible.
- **The HIPAA Risk Analysis** (§164.308(a)(1)(ii)(A)) — the single most-cited deficiency in OCR enforcement, and absent here. Encryption under HIPAA is "addressable," not "required," which means you need a documented risk-based decision, not just the control.
- **BAA with the cloud provider** before any PHI touches it.
- **FIPS 140-3 validated crypto** if this is genuinely federal — Ubuntu Pro FIPS mode, and note that enabling it *restricts* your cipher choices and can conflict with the modern-TLS advice above. That conflict needs resolving in the policy, not at 2am during a deploy.
- FedRAMP / CMMC / HITRUST scoping if applicable; DISA STIG is mentioned once (`:21`) but never used as a baseline.

### 3.19 Evidence generation should be automatic

`:177-183` lists evidence to "collect." Make it a byproduct of the pipeline instead: every deploy writes its OpenSCAP result, image SBOM, signature verification, and config hash to an append-only evidence bucket, keyed by commit SHA. Then an audit request is a query, not a scramble. Add `etckeeper` (the doc mentions it) with the git remote off-host.

### 3.20 Operational conflicts the doc does not warn about

Scripting these blind will burn a day each:

| Control | Breaks |
|---|---|
| `noexec` on `/tmp` | Some `apt`/`pip`/installer paths, Java temp extraction |
| `net.ipv4.ip_forward=0` | Docker (it re-enables it) — see §3.7 |
| `userns-remap` | `--privileged`, bind-mount ownership, existing images |
| `ufw` + Docker | Docker's `iptables` rules bypass `ufw` entirely — `DOCKER-USER` is the only hook |
| AppArmor `enforce` on unprofiled services | Silent failures; profile first in `complain`, use `aa-logprof` |
| `disable_ipv6=1` | Some cloud metadata, apt mirrors, and `localhost` resolution |
| `MemoryDenyWriteExecute` | Any JIT — Node, JVM, Python with certain extensions |
| `SystemCallFilter=@system-service` | Occasionally breaks on glibc updates; test per release |
| `kernel.kexec_load_disabled` | Kdump crash collection |
| HSTS `preload` | Any future plain-HTTP subdomain, for months |

---

## Part 4 — Dev vs production

The doc makes no distinction, and this is where most hardening programs actually fail: dev deploys diverge, then prod hardening becomes untested.

**Rule: same code, different variables.** One Ansible/Terraform codebase, environment-specific variable files. If dev uses a different script, dev proves nothing about prod.

**Safe to relax in dev:** MFA requirement on mesh access, backup retention and cross-region copies, HA/multi-AZ, WAF licensing, EDR agent licensing, patch SLA windows, log retention duration, session recording.

**Must NOT be relaxed in dev** — these are the ones that cause incidents:

- **No production data in dev. Ever.** Synthetic PHI only. A dev database populated from a prod snapshot is a HIPAA breach with extra steps, and it is the most common finding in this class of environment.
- Secret handling — dev secrets still come from the vault, still never land in env vars, still are not committed. Different values, same mechanism.
- Egress default-deny — if dev egress is open, you never discover the allowlist gaps until prod.
- TLS everywhere — use a private CA or `mkcert` for dev, not plaintext. `verify=False` in dev code has a way of shipping.
- No SSH from the internet, no long-lived cloud keys, no shared accounts.
- Image signing and provenance verification — otherwise the verify step is only ever exercised in prod.

**Add a third tier:** a staging environment that is byte-identical to prod's hardening, where you test patches and hardening changes before they reach prod. §3.14's "test patches in a staging VM" needs somewhere to happen.

---

## Part 5 — Suggested build order for the scripting

Ordered by risk reduction per unit of effort, not by the doc's own H/M/L (which puts SSH hardening at [H] and egress filtering nowhere).

**Phase 1 — remove standing trust (highest value, do first)**
1. Remove public SSH; SSM/mesh access only. Break-glass documented and alerted.
2. SSH CA with short-lived certs; `AuthorizedKeysFile none`.
3. IMDSv2 required, hop limit 1, account-level default.
4. Egress default-deny + explicit proxy, with denial alerting.
5. Cloud role attachment — delete every long-lived access key.

**Phase 2 — host baseline (mechanical, fully scriptable)**
6. Golden image pipeline (Packer + CIS base), signed, scanned.
7. Full sysctl + module blacklist set (§3.7).
8. systemd sandboxing on every service + `systemd-analyze security` gate in CI.
9. `unattended-upgrades` on `-security` origins + Livepatch + `needrestart` + orchestrated reboots.
10. Filesystem mount options, TPM2-sealed LUKS, GRUB password.

**Phase 3 — application edge**
11. TLS policy (1.2+/1.3, no 0-RTT, no static tickets), full header set, cookie flags.
12. Rate/connection limits, timeouts, body caps, CDN with origin lock.
13. DB: `hostssl` + SCRAM/cert, non-superuser app role, `pgaudit`, RLS.
14. Field-level encryption for PHI columns, keys in KMS.

**Phase 4 — detection and evidence**
15. Remote-first log shipping, journald sealing, S3 Object Lock, NTS time.
16. `auditd` with a maintained ruleset (`-e 2`), `sudo` I/O logging to `logsrvd`.
17. Falco/Tetragon runtime detection, AIDE with off-host DB, osquery inventory.
18. SIEM with correlation rules and *tested* alerts — write the detection, then trigger it deliberately and confirm it fires.

**Phase 5 — pipeline and governance**
19. OIDC-federated CI, ephemeral runners, SHA-pinned actions, SBOM + Cosign, verify at deploy.
20. OpenSCAP/goss/testssl gates on every deploy; external port scan on a schedule.
21. Control matrix (NIST ID → task → evidence), risk analysis, patch SLAs, IR runbook with a tabletop exercise.
22. Immutable backups with separated credentials, timed restore drills with recorded numbers.

---

## Recommended edits to the source doc

Minimum, before anyone scripts from it:

1. Replace the `daemon.json` block (§1.1) — it is currently a host outage.
2. Fix the `DOCKER-USER` ordering (§1.2) and the `universe` patching advice (§1.3).
3. Remove `node --security` (§1.4) and the corrections in §1.5.
4. Strip or restore the dead `[40]`/`[75]` citation markers and put real URLs in References.
5. Add the frontmatter block (Title | Summary | Tags) that `Research/README` says the future index will consume.
6. Add a prominent scope note distinguishing "hardened perimeter" (what the doc currently describes) from "zero trust" (Part 2), so nobody mistakes the checklist for the latter.

---

## References

Sources verified for the Part 1 findings (checked 2026-08-11):

- Docker Engine v28 default-drop of unpublished ports, and `--ip-forward-no-drop` as the documented opt-out — [Docker Engine v28: Hardening Container Networking by Default](https://www.docker.com/blog/docker-engine-28-hardening-container-networking-by-default/), [moby#49536](https://github.com/moby/moby/issues/49536)
- `disable-legacy-registry` removed in 17.12; `dockerd` fails to start when present — [moby#35751](https://github.com/moby/moby/pull/35751), [Deprecated Docker Engine features](https://docs.docker.com/engine/deprecated/), [DockerCE-CIS#2](https://github.com/florianutz/DockerCE-CIS/issues/2)
- Docker Content Trust / Notary v1 retirement, `notary.docker.io` shutdown 8 Dec 2026 — [Docker Content Trust: Retirement and Migration Guidance](https://www.docker.com/blog/docker-content-trust-retirement-and-migration-guidance/), [Retiring Docker Content Trust](https://www.docker.com/blog/retiring-docker-content-trust/), [Deprecated and retired Docker products](https://docs.docker.com/retired/)
- Node permission model: `--experimental-permission` (20/22) → `--permission` (23.5+); no `--security` flag exists — [Node.js Permissions API](https://nodejs.org/api/permissions.html)

Part 3 recommendations are drawn from Ubuntu/CIS/DISA STIG baselines, NIST SP 800-207 (zero trust) and SP 800-53, systemd unit documentation, and cloud-provider metadata-service hardening guidance. Where a Part 3 snippet gives exact syntax (TPM2 PCR selection, chrony NTS directives, systemd call filters), **verify against the version you deploy** — the §3.15 CI verification gates exist precisely to catch drift in these.
