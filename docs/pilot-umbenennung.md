# Buzz Pilot umbenennen: Bestandsaufnahme und Migrationsplan

Datum: 28.09.2026 · Erstellt von: repo-buzz (Repo-Agent, Buzz) · Auftrag: Christian, eingestellt von Claude
Status: Planung. Namensentscheid steht aus; Code- und Pfadänderungen erfolgen erst danach.
Kein Teil dieses Plans wird ohne Christians Freigabe gemergt oder deployt.

## Hintergrund

In der Software-Fabrik heißt der Zusteller der Aufträge künftig **Dispatcher**
(bisher „Prime-Pilot“), die Fabrik heißt **Factory**. Der Name „Buzz Pilot“
stammt aus der Pilotphase von Buzz und bezeichnet heute die Buzz-Anbindung der
Fabrik: Identitäten, Kanäle, Repo-Registry, `bootstrap`, `provision`,
`project-buzz` und die Repo-Agent-Einstiege. Dieses Dokument erfasst, wo der
Name technisch hängt, schlägt Namen vor und beschreibt einen Migrationsplan ohne
Stillstand.

## 1. Bestandsaufnahme

### 1.1 Eigenes Repository (nicht Teil des Buzz-Quellbaums)

- `pilot/` im Buzz-Laufzeit-Checkout (`/Users/cschroeder/Github/buzz/pilot`) ist
  ein **eigenes Git-Repository**: `git@sgit:ivu/buzz-pilot.git` (Forgejo,
  Remote `forgejo`). Es ist im Buzz-Repo **git-ignored und untracked**
  (`git ls-files pilot` → 0 Dateien). `buzz-custom` enthält `pilot/` nicht.
- Die Release-Pipeline leitet den Release-Verzeichnisnamen daraus ab:
  `roster-<codeapp_sha12>-<buzz_pilot_sha12>` (`prepare_release.py`,
  `_source_commit(buzz, "buzz-pilot")`). Registry-Schlüssel:
  `buzz_pilot_commit`, `buzz_pilot_tree`, Env `TMUXREMOTE_FIRSTMATE_BUZZ_PILOT_COMMIT`.

### 1.2 Verzeichnis `pilot/`

- `bin/` – über 40 Einstiegsskripte: `bootstrap`, `provision`, `project-buzz`,
  `start`, `status`, `repo-agent-supervisor-service`,
  `repo-command-router-entry`/`-service`, `repo-git-watch`, `firstmate-*`,
  `fleet-mcp`, `publish-*`, `public-edge`, `relay-entry`, `update-server-components`.
- `repos/` – Repo-Registry, ca. 30 `*.env`-Dateien (eine pro betreutes Repo:
  `buzz.env`, `codeapp.env`, `ivu-*.env`, `dzg-*.env`, …).
- `groups/` (Kanal-/Gruppenkonfiguration), `prompts/` (Repo-Agent-, Fleet-,
  Heartbeat-Prompts), `tools/` (`repo-git-lock.py`, `scaffold-project-docs.sh`, …),
  `tasks/`, `docs/`, `tests/`.
- Python-Einstiege: `firstmate_acp.py` (FirstMate-Harness),
  `repo_command_router.py` (Kommando-Router).
- `compose.yml` (Relay-Stack der Anbindung).

### 1.3 Umgebungsvariablen

Zwei Namensebenen, zentral definiert in `pilot/bin/common` (gemessen über den `pilot/`-Baum: ~110 `PILOT_*`- und ~30 `BUZZ_PILOT_*`-Namen):

- `PILOT_*` – Arbeitsvariablen, ~110 Namen. Größte Cluster:
  `PILOT_REPO_*` (Registry: `PILOT_REPO_ID`, `PILOT_REPO_WORKSPACE`,
  `PILOT_REPO_CHANNEL`, `PILOT_REPO_GIT_REMOTE`, `PILOT_REPO_AUTHORITY`, …),
  `PILOT_GROUP_*`, `PILOT_AGENT_*` (`PILOT_AGENT_PUBLIC_KEY`,
  `PILOT_AGENT_PRIVATE_KEY`, `PILOT_AGENT_AUTH_TAG`), `PILOT_EDGE_*`,
  `PILOT_STANDUP_*`, `PILOT_BUZZ_*`, `PILOT_RELAY_*`, `PILOT_FLEET_*`.
- `BUZZ_PILOT_*` – aufruferseitige Konfiguration, ~30 Namen, z. B.
  `BUZZ_PILOT_STATE_DIR`, `BUZZ_PILOT_ROOT`, `BUZZ_PILOT_REPO_CONFIG_DIR`,
  `BUZZ_PILOT_GROUP_CONFIG_DIR`, `BUZZ_PILOT_INTEGRATION_ROOT`,
  `BUZZ_PILOT_CODEAPP`, `BUZZ_PILOT_PRIME_CONFIG`, `BUZZ_PILOT_WORKTREE_ROOT`.

Wichtigste Definition (`pilot/bin/common`):

```sh
PILOT_STATE_ROOT="${BUZZ_PILOT_STATE_DIR:-${HOME}/Library/Application Support/Buzz Pilot}"
```

### 1.4 Zustandsverzeichnis

`~/Library/Application Support/Buzz Pilot/` mit u. a.: `identities/`
(Nostr-Identitäten je Agent/Rolle), `channels/` + `channels.env`,
`fleet-keys/`, `repos`-Bezug über `BUZZ_PILOT_REPO_CONFIG_DIR`, `logs/`,
`protocol-publishes/` und `protocol-turns/` (Nachweis-Journal),
`git-readiness/` (u. a. `deployment.json`), `probes/`, `recovery-prechecks/`,
`roster-generation`, `command-router.sqlite3`, `infra.env`, `relay.env`,
`fleet.env`, `daily-upstream/`, `deploy-backups/`, `server-updates/`.

### 1.5 LaunchAgents

| Plist | Bezug zu „Pilot“ |
|---|---|
| `com.cschroeder.buzz-repo-agent-supervisor.plist` | Programm: `…/releases/roster-*/buzz/pilot/bin/repo-agent-supervisor-service`; verweist auf `fleet-jobs/prime_agent_pilot.py` und `tmuxRemote/prime-agent-pilot.json` |
| `com.cschroeder.buzz-repo-command-router.plist` | Argumente: `…/releases/roster-*/buzz/pilot/groups` und `…/buzz/pilot/repos` |
| `com.cschroeder.prime-agent-pilot.plist` | Gleiches Muster; Label trägt „pilot“ |
| `com.cschroeder.buzz-firstmate.plist` | Logs: `~/Library/Logs/BuzzPilot/firstmate.log` |
| `com.cschroeder.buzz-fleet-agent.plist` | Logs: `~/Library/Logs/BuzzPilot/fleet-agent.log` |
| `com.buzz.repo-git-readiness.plist` | Programm: `/Users/cschroeder/Github/buzz/pilot/bin/repo-git-watch` |

Logs liegen unter `~/Library/Logs/BuzzPilot/`.

### 1.6 Release-Kopien

`~/.local/share/tmuxremote/releases/roster-*/buzz/pilot/` – Dutzende
Release-Schnappschüsse von `pilot/`. Die aktiven Releases sind in den
LaunchAgents fest verdrahtet (z. B. `roster-f591bfecff77-*`,
`roster-50efcd0bc05e-*`, `roster-5b9cfd2c1318-*`).

### 1.7 Aufrufer in codeapp (`cwschroeder/codeapp`, Forgejo, Branch `master`)

- `prepare_release.py` – Release-Bau, Quell-Keys `buzz-pilot`/`buzz_pilot`.
- `prepare_services.py` – erzeugt LaunchAgents, verdrahtet
  `prime-agent-pilot`-Job (`fleet-jobs/prime_agent_pilot.py`), Pfade in
  `pilot/bin`, `pilot/groups`, `pilot/repos`, `pilot_load_repo_config`,
  `pilot_worker_pool_effective`.
- `agent_fleet/agent_sync.py` + Tests – Vorgabe
  `~/Library/Application Support/Buzz Pilot` (überschreibbar).
- `scripts/repo-agent-status.py`, `scripts/gitlab_mr_monitor.py`,
  `scripts/deploy-micro-bridge.sh`, `scripts/com.cschroeder.micro-bridge.plist`.
- `fleet-jobs/seat_ledger.py`, `deployment-contract.json`, `docs/DEPLOYMENT.md`,
  `docs/LEARNINGS.md`, diverse `docs/superpowers/*` und `tasks/*` (historisch).

### 1.8 Plugin `buzz-agent-plugin` (`sgit:ivu/buzz-agent-plugin`)

- `plugins/buzz-comms/scripts/project-buzz` – distributbares Gegenstück zum
  Pilot-Helfer; nennt den owner-seitigen Helfer „pilot“.
- **Protokollmarker `[PILOT-…]`** (z. B. `[PILOT-TASK:…]`,
  `[PILOT-FOLDER:…]`) sind Teil des Nachrichtenvertrags:
  `FORBIDDEN_IN_CONTENT = ("[PILOT-", "[AGENT-")`; die Tests
  `test_start_marker_matches_the_pilot_protocol` u. a. binden das Präfix.

### 1.9 tmuxRemote

- `~/Library/Application Support/tmuxRemote/prime-agent-pilot.json` (+ `.lock`),
  Session-Namen wie `firstmate-prime-pilot`, `repo-codeapp-prime-pilot`.
- Bridge-/FirstMate-Konfigurationen unter
  `~/.local/share/tmuxremote/firstmate/configs/…` und
  `~/.local/state/tmuxremote-deploys/…/bridge-config.candidate.json`.

### 1.10 Benennungsrelevant, aber unverändert

- Message-Protokollmarker `[PILOT-` (Vertrag mit Plugin und Projekten).
- Historische Dokumentation (`docs/LEARNINGS.md` buzz + codeapp, `docs/superpowers/*`):
  bleibt als Chronik stehen, wird nicht umgeschrieben.

## 2. Namensvorschläge

Gemeint ist die Buzz-Anbindung der Fabrik (Identitäten, Kanäle, Registry,
Agent-Einstiege) – nicht der Dispatcher selbst, der als Zusteller in codeapp
läuft.

| Variante | Verzeichnis / Repo | Env | Pro | Contra |
|---|---|---|---|---|
| **A: `factory-link`** (Empfehlung) | `factory-link/`, `ivu/buzz-factory-link` | `FACTORY_LINK_*`, `BUZZ_FACTORY_LINK_*` | Beschreibt die Funktion präzise (Anbindung der Factory an Buzz); keine Kollision mit dem Dispatcher-Begriff; Agent-/Kanal- und Registry-Semantik passt | Längerer Name; neuer Compound |
| **B: `dispatcher`** | `dispatcher/`, `ivu/buzz-dispatcher` | `DISPATCHER_*`, `BUZZ_DISPATCHER_*` | Durchgängige Rollensprache mit der neuen Fabrik-Terminologie | Verwechslungsgefahr: der Dispatcher (Zusteller) läuft in codeapp; zwei verschiedene Dinge hießen dann fast gleich |
| **C: `buzz-bridge`** | `buzz-bridge/`, `ivu/buzz-bridge` | `BRIDGE_*`, `BUZZ_BRIDGE_*` | Kurz, selbsterklärend als Brücke Buzz↔Factory | „bridge“ ist im Ökosystem schon vielfach belegt (micro-bridge, agent-bridge, E2E-Bridge); zu generisch |

**Empfehlung: Variante A (`factory-link`).** Sie benennt die Rolle exakt
(Anbindung, nicht Zusteller) und vermeidet die Dispatcher-Kollision. Christian
entscheidet.

## 3. Migrationsplan (Alias-first, ohne Stillstand)

Grundsatz: Neue Namen werden zuerst **neben** den alten gültig (Alias),
dann folgen die Aufrufer, zuletzt verschwinden die alten Namen. Jede Phase ist
einzeln rückrollbar. Codeänderungen starten erst nach Christians Namensentscheid;
Merge und Deploy nur mit Christians Freigabe.

### Phase 1 – Aliase einführen (kein Verhaltenwechsel)

- **Repo/Verzeichnis:** `ivu/buzz-pilot.git` im Forgejo umbenennen (Redirect
  bleibt aktiv) oder neu anlegen und spiegeln; Arbeits-Clone vorerst unter
  `pilot/` lassen. Zielname z. B. `ivu/buzz-factory-link.git`.
- **Env:** `pilot/bin/common` liest zusätzlich die neuen Namen
  (`FACTORY_LINK_STATE_ROOT` o. ä.); Präzedenz: neuer Name gewinnt, sonst
  Fallback auf `BUZZ_PILOT_*`/`PILOT_*`. Alle ~130/50 Variablen nur über die
  Alias-Tabelle in `common` deklarieren, nicht einzeln anfassen.
- **State-Dir:** `common` akzeptiert neuen Root
  (`~/Library/Application Support/Factory Link`); fehlt er, weiter alter Root.
  Kein Umzug in dieser Phase.
- **Protokollmarker:** Plugin (`project-buzz`) und Pilot-Einstiege akzeptieren
  zusätzlich `[FACTORY-LINK-` (neu) neben `[PILOT-` (alt). Alt bleibt gültig.
- **Betroffene Dienste:** alle, aber unverändert konfiguriert – Repo-Agenten,
  FirstMate, Fleet-Agent, Kommando-Router, Relay-Stack.
- **Prüfung:** `pilot/tests` (pytest), Smoke-Turn FirstMate → Repo-Agent,
  Relay-Health-Probe, ein Repo-Agent-Zyklus mit Auftragsquittung.
- **Rückweg:** Alias-Zeilen aus `common` bzw. Plugin-Prüfung entfernen; da
  nichts umgezogen ist, ist der Zustand identisch zum Ausgangspunkt.

### Phase 2 – Aufrufer umstellen

- **codeapp:** `prepare_services.py`/`prepare_release.py` auf neue Pfade und
  Namen umstellen; Registry-Schlüssel `buzz_pilot_commit` →
  `buzz_dispatcher_commit` (oder gewählter Name), alter Schlüssel parallel
  weitergeschrieben; `agent_sync.py`-Default und `repo-agent-status.py` umziehen;
  `prime_agent_pilot.py` → `factory_link_dispatcher.py` (Job-Name neu).
- **LaunchAgents:** neue Plists mit neuem Label (z. B.
  `com.cschroeder.buzz-factory-link`) über `prepare_services.py` bauen und
  laden; alte Plists entladen erst, wenn die neuen laufen.
- **State-Dir-Umzug:** neues Verzeichnis anlegen, Inhalte kopieren (nicht
  verschieben), alter bleibt als Fallback stehen; `logs/BuzzPilot` →
  `logs/FactoryLink` (neu daneben).
- **Release-Kopien:** ein neuer Release-Schnitt
  `roster-<codeapp>-<factory_link>` mit neuem Pfad
  `releases/*/buzz/factory-link`; alte Releases bleiben liegen und dienen als
  Rückweg.
- **tmuxRemote:** `prime-agent-pilot.json` → neue Config-Datei; Session-Namen
  in `factory-link-*` bei natürlicher Neuerstellung (bestehende Sessions
  erhalten, keine Zwangsrenaming laufender Terminals).
- **Betroffene Dienste:** LaunchAgents (Supervisor, Command-Router,
  Prime-Job), codeapp-Pipeline, Plugin-Texte.
- **Prüfung:** voller Nightly-Zyklus (Deployment über `prepare_release` →
  LaunchAgents laden → Repo-Agenten arbeiten), Prüfung
  `repo-agent-status.py`, Git-Readiness-Report, `protocol-publishes` wächst.
- **Rückweg:** alte Plists `launchctl` laden, die auf die letzte
  `roster-*-buzz/pilot`-Release zeigen; State-Fallback greift automatisch.

### Phase 3 – Alte Namen entfernen

- Erst nach einem Beobachtungsfenster (mindestens ein voller Nightly- und
  Arbeitszyklus) ohne Aliasnutzung im Log.
- Alias-Tabelle in `common` auf Fehler umstellen: alter Name gesetzt → lauter
  Fehler („umbenannt, nutze FACTORY_LINK_*“), nicht stillschweigendes Weiterlaufen.
- Altes State-Verzeichnis archivieren (`deploy-backups`-Konvention), nicht
  löschen. Alte Release-Kopien nach Roster-Räumroutine entfernen.
- Marker `[PILOT-` nur entfernen, wenn alle Projekte im Plugin-Protokoll
  umgestellt sind; andernfalls dauerhaft als Legacy-Akzeptanz belassen und das
  hier dokumentieren.
- **Betroffene Dienste:** sämtliche Einstiege (Fehlerpfad!), Plugin, Doku.
- **Prüfung:** `grep -r 'PILOT_\|Buzz Pilot'` über die aktiven Pfade → nur
  noch historische Treffer; pytest-Listen pilot + plugin; ein voller Zyklus.
- **Rückweg:** Alias-Tabelle reaktivieren (Git-History des Pilot-Repos).

## 4. Offene Entscheidungen (Christian)

1. **Name wählen:** Empfehlung Variante A `factory-link`; Alternativen B
   `dispatcher`, C `buzz-bridge` (Abschnitt 2).
2. **Forgejo:** bestehendes Repo `ivu/buzz-pilot` umbenennen (Redirect aktiv)
   oder neues Repo anlegen?
3. **Protokollmarker:** `[PILOT-` als dauerhafte Legacy-Akzeptanz behalten oder
   mit fester Frist abschalten?
4. **Release-Schlüssel:** `buzz_pilot_commit` in der Release-Registry nur
   umbenennen oder dauerhaft parallel schreiben?

## 5. Abschlusskriterium

Der Auftrag ist mit diesem Dokument erfüllt: Bestandsaufnahme (Abschnitt 1),
Namensvarianten mit Empfehlung (Abschnitt 2) und Migrationsplan (Abschnitt 3)
liegen versioniert im Buzz-Repo. Umsetzung startet nach Namensentscheid als
eigener Auftrag, Phase für Phase mit den je angegebenen Prüfungen; Merge und
Deploy jedes Schritts nur mit Christians Freigabe.
