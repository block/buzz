#!/usr/bin/env bash

readonly RELEASE_TAG_RULESET_ID=14378754

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
}

require_release_tag_ruleset() {
  local ruleset_endpoint state can_bypass rule_types includes excludes

  command -v gh >/dev/null 2>&1 || fail_release_ruleset "gh is required" || return 1
  ruleset_endpoint="repos/block/buzz/rulesets/$RELEASE_TAG_RULESET_ID"

  state="$(gh api "$ruleset_endpoint" --jq .enforcement)" || \
    fail_release_ruleset "could not verify Release tag ruleset $RELEASE_TAG_RULESET_ID" || return 1
  [[ "$state" == "active" ]] || \
    fail_release_ruleset "Release tag ruleset $RELEASE_TAG_RULESET_ID is '$state'" || return 1

  can_bypass="$(gh api "$ruleset_endpoint" --jq .current_user_can_bypass)" || \
    fail_release_ruleset "could not verify the release App's tag-ruleset bypass" || return 1
  [[ "$can_bypass" == "always" ]] || \
    fail_release_ruleset "release App cannot always bypass Release tag ruleset $RELEASE_TAG_RULESET_ID (reported '$can_bypass')" || return 1

  rule_types="$(gh api "$ruleset_endpoint" --jq '[.rules[].type] | sort | join(",")')" || \
    fail_release_ruleset "could not verify Release tag ruleset $RELEASE_TAG_RULESET_ID rules" || return 1
  [[ "$rule_types" == "creation,deletion,non_fast_forward,update" ]] || \
    fail_release_ruleset "Release tag ruleset $RELEASE_TAG_RULESET_ID has unexpected rules: '$rule_types'" || return 1

  includes="$(gh api "$ruleset_endpoint" --jq '[.conditions.ref_name.include[]] | sort | join(",")')" || \
    fail_release_ruleset "could not verify Release tag ruleset $RELEASE_TAG_RULESET_ID scope" || return 1
  [[ ",$includes," == *",refs/tags/mobile-v*,"* ]] || \
    fail_release_ruleset "Release tag ruleset $RELEASE_TAG_RULESET_ID does not include refs/tags/mobile-v*" || return 1

  excludes="$(gh api "$ruleset_endpoint" --jq '[.conditions.ref_name.exclude[]] | sort | join(",")')" || \
    fail_release_ruleset "could not verify Release tag ruleset $RELEASE_TAG_RULESET_ID exclusions" || return 1
  [[ -z "$excludes" ]] || \
    fail_release_ruleset "Release tag ruleset $RELEASE_TAG_RULESET_ID has unexpected exclusions: '$excludes'" || return 1
}
