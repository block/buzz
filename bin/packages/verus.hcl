description = "Verus verifier (Rust 1.98.1 via rustup, bundled vstd and Z3 4.16.0)"
repository = "https://github.com/verus-lang/verus"
binaries = ["verus", "cargo-verus"]
strip = 1
version "0.2026.10.04.426d8b0" {
  platform "linux" "amd64" {
    source = "https://github.com/verus-lang/verus/releases/download/release/0.2026.10.04.426d8b0/verus-0.2026.10.04.426d8b0-x86-linux.zip"
    sha256 = "624018662868563a7bec0dd07be5e4dd0c3d61124b4b55105765884bf2d4d1a0"
  }
  platform "darwin" "amd64" {
    source = "https://github.com/verus-lang/verus/releases/download/release/0.2026.10.04.426d8b0/verus-0.2026.10.04.426d8b0-x86-macos.zip"
    sha256 = "9b00357ee298a3b052038abc85c0be3129ab2821762fe8be5eb8dcbe37349f9c"
  }
  platform "darwin" "arm64" {
    source = "https://github.com/verus-lang/verus/releases/download/release/0.2026.10.04.426d8b0/verus-0.2026.10.04.426d8b0-arm64-macos.zip"
    sha256 = "83cafc50284caea07fa8762df04b00cc0af441f25cdf1631d6c30987c65af446"
  }
}
