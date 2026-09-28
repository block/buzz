# Bestie Scratch

Lightweight personal pad in the Bestie DM RHS (localStorage, scoped by relay /
owner). Not a wiki — title + body notes only.

Mutations:

1. **RHS UI** — Scratch category list (title/snippet); click opens the editor
   alone (list hidden until Close/Save); `+` to add; delete on the row.
2. **Natural language** — `park this: …`, `save to scratch: Title\nbody`, or
   `scratch: …` (client-side).
3. **Agent fence** — optional:

```bestie-scratch
{"op":"add","title":"Idea","body":"park this for later"}
```

Ops: `add`, `update`, `remove`.
