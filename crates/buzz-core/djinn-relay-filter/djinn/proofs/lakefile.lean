import Lake
open Lake DSL

require aeneas from "/home/bloxer/rust-djinn/tools/aeneas/backends/lean"

package «djinn-relay-filter» {}

@[default_target] lean_lib «RelayFilter» {}
