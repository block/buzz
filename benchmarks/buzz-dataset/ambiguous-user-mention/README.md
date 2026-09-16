# ambiguous-user-mention

The channel contains two real identities with the exact same three-word display
name. Their profile `about` fields carry different routing codes. The agent must
discover the intended pubkey, notify it exactly once, never notify the twin,
and separately callback the requester. This guards the silent ambiguity family
reported in [block/buzz#4303](https://github.com/block/buzz/issues/4303) and
[block/buzz#6257](https://github.com/block/buzz/issues/6257).
