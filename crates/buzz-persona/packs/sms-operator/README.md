# SMS Operator persona pack

A single-persona pack for the Buzz ↔ Twilio SMS integration (see
`docs/wayfinder/map-external-project-sms-integration.md`). Subscribed to the
SMS inbox channel, it resolves each inbound text's `project` tag to dispatch
into the right external repo, or asks the sender to pick one when it's
unresolved.

## Deploying

```bash
buzz-acp --pack crates/buzz-persona/packs/sms-operator
```

`subscribe: ["sms-inbox"]` in `agents/sms-operator.persona.md` is a
placeholder — override it with `--channels <channel-id-or-name>` at deploy
time if your community's private SMS-inbox channel has a different name.

## Validating

```bash
buzz pack validate crates/buzz-persona/packs/sms-operator
buzz pack inspect crates/buzz-persona/packs/sms-operator
```
