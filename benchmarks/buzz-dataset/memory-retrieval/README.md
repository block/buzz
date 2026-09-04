# memory-retrieval

Before the agent starts, the harness runs `buzz mem set` with the agent's own
Buzz credentials to seed five similar cold memories. One records the exact
total customer count for April 2024; the other four contain customer counts for
nearby months or related April metrics. The harness then delivers
`instruction.md`, which contains only the retrieval question and does not reveal
the answer or memory slug. No channel message contains the answer, so
conversation history cannot supply it.

Full credit requires the exact customer count `352,345` in the threaded answer.
Equivalent comma-free formatting is accepted, but rounded or approximate counts
receive no credit. Credit is also voided if the answer mentions another number,
apart from the requested year `2024`. This includes every count drawn from the
distractor memories, so dumping several memories or selecting the wrong one does
not pass — the answer must resolve to the correct value alone. The verifier does
not inspect tool calls: seeding is deterministic harness setup, and retrieval is
graded only through the observable answer.
