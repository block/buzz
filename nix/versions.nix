{
  # buzz-acp/buzz-agent/buzz-cli don't carry an independent release version —
  # the whole workspace shares `workspace.package.version = "0.1.0"` — so a
  # commit SHA is the only meaningful pin for this sidecars-only build.
  # Bump by hand; `nix build` reports the correct new hash on mismatch.
  buzzRev = "dad5a33865fc81a2e55b3b60746632f615ec1e3a";
  buzzSrcHash = "sha256-5Y6KhzrpGUb7tw71t3bN5asJg45rNz413gNt0m4lPks=";

  # Cargo.lock entries sourced from git rather than crates.io.
  # buildRustPackage needs an explicit FOD hash for each distinct git
  # source. Regenerate by deleting an entry and letting `nix build` report
  # the correct hash on mismatch.
  sidecarCargoOutputHashes = {
    "aws-creds-0.39.1" = "sha256-QAAm1phmeLFtDRgfDCoHijN1ce/rYzh18KziOUbL+hw=";
    "mesh-llm-api-client-0.75.1" = "sha256-RXjmM66u40cxnacbvTtCFJShMK4BM+MHOyJ2vQ7Gw60=";
  };
}
