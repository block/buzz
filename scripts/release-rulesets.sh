#!/usr/bin/env bash

# Tag ruleset ("Restrict Mobile Tags") that protects mobile candidate tags. Its
# bypass actors are the mobile release managers, never the buzz-release-bot App:
# operators push candidate tags themselves.
readonly MOBILE_RELEASE_TAG_RULESET_ID=26588677

fail_release_ruleset() {
  echo "Error: $*" >&2
  return 1
}

require_canonical_repository() {
  local origin_url

  origin_url="$(git config --get remote.origin.url 2>/dev/null)" || \
    fail_release_ruleset "origin is required and must point to block/buzz" || return 1
  case "$origin_url" in
    git@github.com:block/buzz.git|ssh://git@github.com/block/buzz.git|https://github.com/block/buzz.git|https://github.com/block/buzz)
      ;;
    *)
      fail_release_ruleset "origin must point to canonical block/buzz, not '$origin_url'" || return 1
      ;;
  esac
  # Tags are pushed with `git push origin`, which honors remote.origin.pushurl
  # and url.*.pushInsteadOf. Require the effective push destination to be the
  # same canonical repository that origin fetches from.
  [[ "$(git remote get-url --push origin)" == "$(git remote get-url origin)" ]] || \
    fail_release_ruleset "origin pushes somewhere other than it fetches from; remove remote.origin.pushurl or pushInsteadOf so candidate tags reach canonical block/buzz" || return 1
}

require_mobile_release_tag_ruleset() {
  local ruleset_endpoint state can_bypass rule_types includes excludes

  [[ "$MOBILE_RELEASE_TAG_RULESET_ID" =~ ^[0-9]+$ ]] || \
    fail_release_ruleset "MOBILE_RELEASE_TAG_RULESET_ID in scripts/release-rulesets.sh must be numeric (got '$MOBILE_RELEASE_TAG_RULESET_ID')" || return 1
  command -v gh >/dev/null 2>&1 || fail_release_ruleset "gh is required" || return 1
  ruleset_endpoint="repos/block/buzz/rulesets/$MOBILE_RELEASE_TAG_RULESET_ID"

  state="$(gh api "$ruleset_endpoint" --jq .enforcement)" || \
    fail_release_ruleset "could not verify mobile tag ruleset $MOBILE_RELEASE_TAG_RULESET_ID" || return 1
  [[ "$state" == "active" ]] || \
    fail_release_ruleset "mobile tag ruleset $MOBILE_RELEASE_TAG_RULESET_ID is '$state'" || return 1

  can_bypass="$(gh api "$ruleset_endpoint" --jq .current_user_can_bypass)" || \
    fail_release_ruleset "could not verify your bypass of mobile tag ruleset $MOBILE_RELEASE_TAG_RULESET_ID" || return 1
  [[ "$can_bypass" == "always" ]] || \
    fail_release_ruleset "you cannot bypass mobile tag ruleset $MOBILE_RELEASE_TAG_RULESET_ID (reported '$can_bypass'); you must be a bypass actor of mobile tag ruleset $MOBILE_RELEASE_TAG_RULESET_ID (currently the mobile release managers)" || return 1

  rule_types="$(gh api "$ruleset_endpoint" --jq '[.rules[].type] | sort | join(",")')" || \
    fail_release_ruleset "could not verify mobile tag ruleset $MOBILE_RELEASE_TAG_RULESET_ID rules" || return 1
  [[ "$rule_types" == "creation,deletion,non_fast_forward,update" ]] || \
    fail_release_ruleset "mobile tag ruleset $MOBILE_RELEASE_TAG_RULESET_ID has unexpected rules: '$rule_types'" || return 1

  includes="$(gh api "$ruleset_endpoint" --jq '[.conditions.ref_name.include[]] | sort | join(",")')" || \
    fail_release_ruleset "could not verify mobile tag ruleset $MOBILE_RELEASE_TAG_RULESET_ID scope" || return 1
  # Either pattern covers every mobile-vX.Y.Z-rc.N candidate tag.
  [[ ",$includes," == *",refs/tags/mobile-*,"* || ",$includes," == *",refs/tags/mobile-v*,"* ]] || \
    fail_release_ruleset "mobile tag ruleset $MOBILE_RELEASE_TAG_RULESET_ID must include refs/tags/mobile-* or refs/tags/mobile-v*" || return 1

  excludes="$(gh api "$ruleset_endpoint" --jq '[.conditions.ref_name.exclude[]] | sort | join(",")')" || \
    fail_release_ruleset "could not verify mobile tag ruleset $MOBILE_RELEASE_TAG_RULESET_ID exclusions" || return 1
  [[ -z "$excludes" ]] || \
    fail_release_ruleset "mobile tag ruleset $MOBILE_RELEASE_TAG_RULESET_ID has unexpected exclusions: '$excludes'" || return 1
}
