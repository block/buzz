manage-git = true
# Keep Cargo's registry, git checkouts, and installed tools outside the
# checkout. The rustup package defaults CARGO_HOME to ${HERMIT_ENV}/.hermit/rust,
# which made Swatinem/rust-cache treat every registry crate as a workspace
# member and save an empty target/ for the root workspace. A shared home also
# lets worktrees reuse one registry. scripts/test-rust-cache-contract.sh pins this.
# ~/.cargo-hermit/bin comes first on PATH, so a `cargo install` from any buzz
# checkout shadows same-named Hermit-pinned tools in every buzz checkout.
env = {
  "CARGO_HOME": "${HOME}/.cargo-hermit",
  "PATH": "${HOME}/.cargo-hermit/bin:${PATH}",
}
