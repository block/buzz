These are unchanged HTTP response bodies exported through the production HTTP
router of block/buzz BUZZ-511 PR #8988 at `d3ae4935b11e207f0736833cd5220f207be359fd`.

The authoritative full producer SHA and regeneration program are recorded in
[the original fixture provenance](https://github.com/block/buzz/blob/082149e503a899daad5711a7815ac86e71fc3a16/mobile/test/fixtures/nip_fi/README.md).
The response bodies cover required, optional, and optional with FI capability.
The exporter verified HTTP 200, root/info agreement and Cache-Control: no-store;
its synthetic policy/verifier boundaries do not establish live acceptance.

Before readiness, re-export from the merged producer SHA and update this pin.
