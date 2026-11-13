#!/usr/bin/env bash
# Model-checks the Git-on-object-storage protocol (docs/git-on-object-storage.md).
#
#   docs/spec/check.sh            run the whole suite
#   docs/spec/check.sh NAME...    run only the named checks; a name that is
#                                 not a check is an error (exit status 2)
#
# The suite has two halves.  The protocol as specified must satisfy every
# property.  Each seeded design error must violate the property named for it;
# a seeded error that goes undetected fails the suite, because it would mean
# the properties do not constrain that part of the design.
# A last check asserts that a behavior of interest never occurs and must be
# violated too, which shows the model reaches that behavior.
#
# Environment:
#   TLA2TOOLS_JAR   path to tla2tools.jar (default: downloaded next to this file)
#   TLC_OUT         directory for TLC output and counterexamples (default: temp)
#   TLC_WORKERS     TLC worker threads (default: auto)
set -euo pipefail

here=$(cd "$(dirname "$0")" && pwd)

# Reject names that are not checks before doing any work.  Otherwise a
# misspelled name selects nothing and the run reports success without having
# checked anything.  The names are read from the `check` lines of this file.
known=$({ echo refdb; sed -nE 's/^check +([a-z0-9-]+) .*/\1/p' "$here/$(basename "$0")"; })
for name in "$@"; do
  if ! printf '%s\n' "$known" | grep -qxF -- "$name"; then
    echo "error: unknown check '$name'" >&2
    echo "known checks: $(printf '%s' "$known" | tr '\n' ' ')" >&2
    exit 2
  fi
done
cd "$here"

TLC_VERSION=1.7.4
TLC_SHA256=936a262061c914694dfd669a543be24573c45d5aa0ff20a8b96b23d01e050e88
jar=${TLA2TOOLS_JAR:-$here/tla2tools.jar}
out=${TLC_OUT:-$(mktemp -d "${TMPDIR:-/tmp}/buzz-tlc.XXXXXX")}
workers=${TLC_WORKERS:-auto}
mkdir -p "$out"

sha256() {
  if command -v sha256sum >/dev/null 2>&1; then sha256sum "$1" | cut -d' ' -f1
  else shasum -a 256 "$1" | cut -d' ' -f1; fi
}

if [ ! -f "$jar" ]; then
  echo "fetching TLC $TLC_VERSION"
  curl -fsSL -o "$jar.part" \
    "https://github.com/tlaplus/tlaplus/releases/download/v$TLC_VERSION/tla2tools.jar"
  mv "$jar.part" "$jar"
fi
if [ "$(sha256 "$jar")" != "$TLC_SHA256" ]; then
  echo "error: $jar is not TLC $TLC_VERSION (sha256 mismatch)" >&2
  exit 2
fi

failures=0
selected=("$@")

wanted() {
  [ ${#selected[@]} -eq 0 ] && return 0
  local n
  for n in "${selected[@]}"; do [ "$n" = "$1" ] && return 0; done
  return 1
}

# tlc NAME MODULE CONFIG -> writes $out/NAME.txt, prints "<verdict> <states>"
#   verdict: "pass", "violated:<property>", or "error"
tlc() {
  local name=$1 module=$2 config=$3 log="$out/$1.txt" verdict states
  java -XX:+UseParallelGC -cp "$jar" tlc2.TLC -workers "$workers" -deadlock \
    -metadir "$out/$name.meta" -config "$config" "$module" >"$log" 2>&1 || true
  rm -rf "$out/$name.meta"
  if grep -q "Model checking completed. No error has been found." "$log"; then
    verdict=pass
  elif grep -q "Error: Invariant .* is violated" "$log"; then
    verdict="violated:$(sed -n 's/^Error: Invariant \(.*\) is violated.*/\1/p' "$log" | head -1)"
  elif grep -q "Error: Action property .* of module GitRefDB is violated" "$log"; then
    # The refinement: a step that is not a step of the abstract specification.
    verdict="violated:Refines"
  elif grep -q "Error: Action property .* is violated" "$log"; then
    verdict="violated:$(sed -n 's/^Error: Action property \(.*\) is violated.*/\1/p' "$log" | head -1)"
  else
    verdict=error
  fi
  states=$(sed -n 's/^\([0-9,]*\) states generated, \([0-9,]*\) distinct states found.*/\2/p' "$log" | tail -1)
  echo "$verdict ${states:-?}"
}

# check NAME EXPECT [sed-expression...]
#   EXPECT is "pass" or the property the configuration must violate.
#   The sed expressions rewrite MCGitOnObjectStore.cfg into this configuration.
check() {
  local name=$1 expect=$2 cfg="$out/$1.cfg" e result verdict states start
  local args=(-e '/^\\\*/d') # drop the comment header of the base configuration
  shift 2
  wanted "$name" || return 0
  for e in "$@"; do args+=(-e "$e"); done
  if [ "$expect" != pass ]; then
    # Check only the property this seeded error is expected to violate, so
    # the verdict names it whatever else the error also breaks.  The state
    # view is dropped: its justification (MCGitOnObjectStore.tla) holds for
    # the protocol as specified, and under a seeded error it can hide the
    # violation (it hides a deleted pack that no manifest in use lists yet).
    args+=(-e '/^INVARIANT /d' -e '/^PROPERTY /d' -e '/^VIEW /d')
  fi
  sed "${args[@]}" MCGitOnObjectStore.cfg >"$cfg"
  case "$expect" in
    pass) ;;
    Refines | ParentIsPredecessor) echo "PROPERTY $expect" >>"$cfg" ;;
    *) echo "INVARIANT $expect" >>"$cfg" ;;
  esac
  start=$(date +%s)
  result=$(tlc "$name" MCGitOnObjectStore.tla "$cfg")
  verdict=${result% *}; states=${result#* }
  report "$name" "$expect" "$verdict" "$states" $(( $(date +%s) - start ))
}

report() {
  local name=$1 expect=$2 verdict=$3 states=$4 secs=$5 want status
  if [ "$expect" = pass ]; then want=pass; else want="violated:$expect"; fi
  if [ "$verdict" = "$want" ]; then status=ok; else status=FAIL; failures=$((failures + 1)); fi
  printf '%-4s %-22s expected %-34s got %-34s %12s states %4ss\n' \
    "$status" "$name" "$want" "$verdict" "$states" "$secs"
}

# Substitutions.
ONE_CLIENT='s/Clients = {c1, c2}/Clients = {c1}/'
THREE_CLIENTS='s/Clients = {c1, c2}/Clients = {c1, c2, c3}/'
ONE_REF='s/Refs = {main, dev}/Refs = {main}/'
TWO_OBJECTS='s/Oids = {o1}/Oids = {o1, o2}/'
NO_SETTER='s/HasSetter = TRUE/HasSetter = FALSE/'
READER='s/HasReader = FALSE/HasReader = TRUE/'
STALE='s/StaleClients = FALSE/StaleClients = TRUE/'
PARENT='s/ParentLinks = FALSE/ParentLinks = TRUE/'
VERSION='s/EtagMode = "content"/EtagMode = "version"/'
CONSTANT_ETAG='s/EtagMode = "content"/EtagMode = "constant"/'
LEGACY='s/Decision = "ground-truth"/Decision = "legacy"/'
REPORT='s/Decision = "ground-truth"/Decision = "report"/'
NO_VIEW='/^VIEW /d'
NO_CRASH='s/GitMayCrash = TRUE/GitMayCrash = FALSE/'
fault() { echo "s/Faults = {}/Faults = {\"$1\"}/"; }

echo "TLC output: $out"
echo
echo "The abstract specification"
if wanted refdb; then
  start=$(date +%s)
  result=$(tlc refdb GitRefDB.tla GitRefDB.cfg)
  report refdb pass "${result% *}" "${result#* }" $(( $(date +%s) - start ))
fi

echo
echo "The protocol as specified (must satisfy every property)"
check push            pass
check objects         pass "$ONE_CLIENT" "$TWO_OBJECTS" "$NO_SETTER" "$READER"
check three-clients   pass "$THREE_CLIENTS" "$ONE_REF" "$NO_SETTER"
check stale-commands  pass "$ONE_CLIENT" "$STALE" "$TWO_OBJECTS" "$NO_SETTER"
check parent-links    pass "$PARENT" "$NO_SETTER"
check version-etag    pass "$VERSION" "$NO_SETTER"
check no-view         pass "$NO_VIEW" "$NO_SETTER"

echo
echo "Seeded design errors (each must violate the property named)"
check legacy-unauthorized  PublishIsAuthorizedChange "$LEGACY" "$NO_SETTER" "$NO_CRASH"
check legacy-silent-drop   NoSilentDrop              "$LEGACY" "$NO_SETTER" "$NO_CRASH"
check legacy-crash-drop    NoSilentDrop              "$LEGACY" "$NO_SETTER"
check legacy-refinement    Refines                   "$LEGACY" "$NO_SETTER"
check report-unauthorized  PublishIsAuthorizedChange "$REPORT" "$NO_SETTER" "$NO_CRASH"
check report-crash-drop    NoSilentDrop              "$REPORT" "$NO_SETTER"
check report-refinement    Refines                   "$REPORT" "$NO_SETTER"
check no-hook              PublishIsAuthorizedChange "$(fault no-hook)" "$NO_SETTER"
check no-cas-guard         Refines                   "$(fault no-cas-guard)" "$NO_SETTER"
check stale-base           Refines                   "$(fault stale-base)" "$NO_SETTER"
check stale-base-chain     ParentIsPredecessor       "$(fault stale-base)" "$NO_SETTER" "$PARENT"
check constant-etag        Refines                   "$CONSTANT_ETAG" "$NO_SETTER"
check constant-etag-setter Refines                   "$CONSTANT_ETAG" "$ONE_CLIENT"
check ack-before-cas       Refines                   "$(fault ack-before-cas)" "$NO_SETTER"
check cas-before-upload    Fence                     "$(fault cas-before-upload)" "$NO_SETTER"
check gc-pending-upload    Fence                     "$(fault gc)" "$ONE_CLIENT" "$TWO_OBJECTS" "$NO_SETTER"
check gc-reader            ReaderComplete            "$(fault gc)" "$ONE_CLIENT" "$TWO_OBJECTS" "$NO_SETTER" "$READER"
check drop-parent-packs    PublishedClosed           "$(fault drop-parent-packs)" "$ONE_CLIENT" "$TWO_OBJECTS" "$NO_SETTER"
check no-connectivity      PublishedClosed           "$(fault no-connectivity)" "$ONE_CLIENT" "$TWO_OBJECTS" "$NO_SETTER"

echo
echo "Coverage (a behavior of the protocol as specified that must be reachable)"
check relayed-ok           NeverRelaysOk             "$NO_SETTER"

echo
if [ "$failures" -ne 0 ]; then
  echo "$failures check(s) did not give the expected result; see $out"
  exit 1
fi
echo "all checks gave the expected result"
