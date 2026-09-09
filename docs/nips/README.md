# Buzz NIP documents

Local protocol proposals; draft status does not imply deployed support.

| Document | Subject |
| --- | --- |
| [NIP-AA](NIP-AA.md) | Agent Authentication |
| [NIP-AE](NIP-AE.md) | Agent Engrams |
| [NIP-AM](NIP-AM.md) | Agent Turn Metrics |
| [NIP-AO](NIP-AO.md) | Agent Observability |
| [NIP-AP](NIP-AP.md) | Agent Personas |
| [NIP-BW](NIP-BW.md) | MyBuzz issue-to-human-acceptance wire contract |
| [NIP-CW](NIP-CW.md) | Channel Window |
| [NIP-DV](NIP-DV.md) | DM Visibility |
| [NIP-ER](NIP-ER.md) | Event Reminders |
| [NIP-GS](NIP-GS.md) | Git Object Signing with Nostr Keys |
| [NIP-IA](NIP-IA.md) | Identity Archival |
| [NIP-MP](NIP-MP.md) | Multi-Repository Projects |
| [NIP-OA](NIP-OA.md) | Owner Attestation |
| [NIP-PL](NIP-PL.md) | Push Notifications |
| [NIP-PMA](NIP-PMA.md) | Private Managed Agents |
| [NIP-RS](NIP-RS.md) | Cross-Device Read State Sync |
| [NIP-WP](NIP-WP.md) | Workspace Profile |

NIP-BW adds the [P1 fixture corpus](NIP-BW.fixtures.json). Validate it with
`python3 scripts/check-nip-bw-fixtures.py` and
`python3 scripts/test-nip-bw-fixtures.py` from the repository root.
These validate documentation inputs, not production workflow behavior.

The 2026-09-09 NIP-BW acceptance correction preserves valid human acceptance
across historical handoffs, technical conflicts and later tests. The corpus
includes both delivery orders, completed sets and independent follow-up issues.
