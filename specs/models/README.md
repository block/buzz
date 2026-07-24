# Domain Model Glossary

| Term | Meaning |
| --- | --- |
| Local relay | A single-process, one-community Buzz node intended for local experimentation. |
| Signed event | A Nostr event whose ID and Schnorr signature verify. |
| Durable event | A signed event outside the NIP-01 ephemeral kind range. |
| Effective event | The event currently visible after replaceable-event rules are applied. |
| Event log | An append-only newline-delimited JSON record of accepted durable events. |
| Subscription | A client-selected set of Nostr filters receiving historical and live matching events. |
| Stable node | A node whose acknowledged durable history survives restart and remains attributable. |
