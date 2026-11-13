# Review lab fixture — do not merge

This draft exists only to test PR-to-lane routing and reviewer replies.
It changes documentation fixtures only and has no runtime integration.
The examples deliberately contain reviewable defects.

The draft has three changed files. Suggested routing rules:

| Lane | Field | Condition | Value |
| --- | --- | --- | --- |
| General | No rules | Match every PR | — |
| Backend | Changed file path | matches | `docs/review-lab-fixture/backend/**` |
| UI | Changed file path | matches | `docs/review-lab-fixture/ui/**` |
| Small changes | Total changed files | at most | `3` |
| Title example | Title | contains | `review lab` |

For each lane, assign a connected reviewer and instructions to inspect only
its fixture. Run the review in the local Buzz lab. Keep findings in the Buzz
lane thread; this draft is not a production change or a request to merge.

Close this draft without merging when testing is complete.
