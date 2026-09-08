#!/usr/bin/env bash
set -euo pipefail

repo_root="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
checker="$repo_root/scripts/check-postgres-test-discovery.py"
fixture_root="$(mktemp -d "${TMPDIR:-/tmp}/buzz-postgres-discovery.XXXXXX")"
trap 'rm -rf "$fixture_root"' EXIT

mkdir -p "$fixture_root/src/standard" "$fixture_root/src/tests" "$fixture_root/tests/common"

cat >"$fixture_root/Cargo.toml" <<'TOML'
[package]
name = "postgres-discovery-fixture"
version = "0.0.0"
edition = "2021"
TOML

cat >"$fixture_root/src/good.rs" <<'RS'
#[cfg(test)]
mod postgres_tests {
    #[test]
    #[ignore = "requires Postgres"]
    fn ordinary_database_test() {}

    mod external_infra_tests {
        #[tokio::test]
        #[ignore = "requires Postgres and S3-compatible storage"]
        async fn hybrid_database_test() {}
    }
}
RS

cat >"$fixture_root/tests/postgres_search.rs" <<'RS'
#[test]
#[ignore = "requires PostgreSQL"]
fn integration_database_test() {}
RS

cat >"$fixture_root/src/lib.rs" <<'RS'
#[cfg(test)]
#[path = "out_of_line.rs"]
mod postgres_tests;

#[cfg(test)]
mod standard;
RS

cat >"$fixture_root/src/out_of_line.rs" <<'RS'
#[test]
#[ignore = "requires PostgreSQL"]
fn out_of_line_database_test() {}
RS

cat >"$fixture_root/src/standard.rs" <<'RS'
mod postgres_tests;
RS

cat >"$fixture_root/src/standard/postgres_tests.rs" <<'RS'
#[test]
#[ignore = "requires PostgreSQL"]
fn standard_out_of_line_database_test() {}
RS

python3 "$checker" "$fixture_root"
python3 "$checker" "$fixture_root/src/out_of_line.rs"
python3 "$checker" "$fixture_root/src/standard/postgres_tests.rs"

packages="$("$repo_root/scripts/postgres-test-packages.sh" "$fixture_root")"
if [[ "$packages" != "postgres-discovery-fixture" ]]; then
  echo "expected fixture package discovery, got: $packages" >&2
  exit 1
fi

mkdir -p "$fixture_root/no-tomllib"
cat >"$fixture_root/no-tomllib/tomllib.py" <<'PY'
raise ModuleNotFoundError("simulate Python 3.9 without tomllib")
PY

fallback_packages="$(
  PYTHONPATH="$fixture_root/no-tomllib" \
    python3 "$checker" --print-packages "$fixture_root"
)"
if [[ "$fallback_packages" != "postgres-discovery-fixture" ]]; then
  echo "expected Python 3.9-compatible package discovery, got: $fallback_packages" >&2
  exit 1
fi

cat >"$fixture_root/src/tests/postgres_nested.rs" <<'RS'
#[test]
#[ignore = "requires PostgreSQL"]
fn nested_source_module_is_not_an_integration_binary() {}
RS

cat >"$fixture_root/tests/common/postgres_helper.rs" <<'RS'
#[test]
#[ignore = "requires PostgreSQL"]
fn nested_integration_helper_is_not_an_integration_binary() {}
RS

if python3 "$checker" "$fixture_root" >"$fixture_root/nested.out" 2>&1; then
  echo "expected nested postgres_* modules to fail discovery validation" >&2
  exit 1
fi
grep -q "nested_source_module_is_not_an_integration_binary" "$fixture_root/nested.out"
grep -q "nested_integration_helper_is_not_an_integration_binary" "$fixture_root/nested.out"
rm "$fixture_root/src/tests/postgres_nested.rs"
rm "$fixture_root/tests/common/postgres_helper.rs"

cat >"$fixture_root/src/external_postgres_only.rs" <<'RS'
#[cfg(test)]
mod postgres_tests {
    mod external_infra_tests {
        #[test]
        #[ignore = "requires PostgreSQL"]
        fn postgres_only_test_cannot_hide_under_external_infra() {}
    }
}
RS

if python3 "$checker" "$fixture_root" >"$fixture_root/external-postgres.out" 2>&1; then
  echo "expected a PostgreSQL-only external-infra test to fail validation" >&2
  exit 1
fi
grep -q "postgres_only_test_cannot_hide_under_external_infra" \
  "$fixture_root/external-postgres.out"
rm "$fixture_root/src/external_postgres_only.rs"

cat >"$fixture_root/src/bare_ignore.rs" <<'RS'
#[cfg(test)]
mod postgres_tests {
    #[test]
    #[ignore]
    fn bare_ignore_in_postgres_module_has_no_classification() {}
}
RS

cat >"$fixture_root/tests/postgres_bare.rs" <<'RS'
#[test]
#[ignore]
fn bare_ignore_in_postgres_binary_has_no_classification() {}
RS

if python3 "$checker" "$fixture_root" >"$fixture_root/bare-ignore.out" 2>&1; then
  echo "expected bare ignores in PostgreSQL structures to fail validation" >&2
  exit 1
fi
grep -q "bare_ignore_in_postgres_module_has_no_classification" \
  "$fixture_root/bare-ignore.out"
grep -q "bare_ignore_in_postgres_binary_has_no_classification" \
  "$fixture_root/bare-ignore.out"
rm "$fixture_root/src/bare_ignore.rs"
rm "$fixture_root/tests/postgres_bare.rs"

cat >"$fixture_root/src/missed.rs" <<'RS'
#[cfg(test)]
mod tests {
    #[test]
    #[ignore = "requires Postgres"]
    fn silently_missed_database_test() {}
}
RS

if python3 "$checker" "$fixture_root" >"$fixture_root/missed.out" 2>&1; then
  echo "expected an unclassified PostgreSQL test to fail discovery validation" >&2
  exit 1
fi
grep -q "silently_missed_database_test" "$fixture_root/missed.out"
rm "$fixture_root/src/missed.rs"

cat >"$fixture_root/src/raw_missed.rs" <<'RS'
#[cfg(test)]
mod tests {
    #[test]
    #[ignore = r#"requires PostgreSQL"#]
    fn raw_string_database_test() {}
}
RS

if python3 "$checker" "$fixture_root" >"$fixture_root/raw-missed.out" 2>&1; then
  echo "expected a raw-string PostgreSQL reason to fail discovery validation" >&2
  exit 1
fi
grep -q "raw_string_database_test" "$fixture_root/raw-missed.out"
rm "$fixture_root/src/raw_missed.rs"

cat >"$fixture_root/src/commented.rs" <<'RS'
#[cfg(test)]
mod tests {
    // #[ignore = "requires Postgres"]
    fn ordinary_helper() {}
}
RS

python3 "$checker" "$fixture_root"
rm "$fixture_root/src/commented.rs"

cat >"$fixture_root/src/hybrid.rs" <<'RS'
#[cfg(test)]
mod postgres_tests {
    #[test]
    #[ignore = "requires Postgres and MinIO"]
    fn hybrid_without_external_module() {}
}
RS

if python3 "$checker" "$fixture_root" >"$fixture_root/hybrid.out" 2>&1; then
  echo "expected a hybrid test without an external-infra module to fail validation" >&2
  exit 1
fi
grep -q "hybrid_without_external_module" "$fixture_root/hybrid.out"
rm "$fixture_root/src/hybrid.rs"

cat >"$fixture_root/src/name_is_not_classification.rs" <<'RS'
#[cfg(test)]
mod postgres_tests {
    #[test]
    #[ignore = "requires Postgres and MinIO"]
    fn external_infra_prefix_is_not_enough() {}
}
RS

if python3 "$checker" "$fixture_root" >"$fixture_root/name.out" 2>&1; then
  echo "expected function-name infrastructure classification to fail validation" >&2
  exit 1
fi
grep -q "external_infra_prefix_is_not_enough" "$fixture_root/name.out"
rm "$fixture_root/src/name_is_not_classification.rs"

python3 "$checker" "$repo_root/crates"

echo "PostgreSQL test discovery convention checks passed"
