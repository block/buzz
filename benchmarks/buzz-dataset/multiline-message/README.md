# multiline-message

The agent sends a short release update whose blank lines and bullet boundaries
must survive the `buzz messages send` shell call as real newline bytes. The
verifier also checks the normal reply anchor and callback mention. This guards
the first-newline truncation failure described in
[block/buzz#5787](https://github.com/block/buzz/issues/5787).

Run with the command in the parent [README](../README.md), replacing the task
path with `benchmarks/buzz-dataset/multiline-message`.
