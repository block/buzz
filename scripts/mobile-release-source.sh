#!/usr/bin/env bash

# Validate an explicitly selected stable branch against the marketing version.
# main remains the default for releases without a stabilization branch.
require_mobile_release_source() {
  local version="$1" branch="$2"
  if [[ "$branch" == main ]]; then
    return 0
  fi
  if [[ "$branch" != "mobile-v${version%.*}" ]]; then
    echo "Source branch must be main or mobile-v${version%.*} for version $version" >&2
    return 1
  fi
}

# The repository administrator must configure this ruleset without bypass actors.
# GitHub hides that list from non-admin tokens; check the caller's effective
# bypass plus every public policy field on each publication.
require_mobile_release_branch_ruleset() {
  local branch="$1" valid
  [[ "$branch" != main ]] || return 0
  valid="$(gh api repos/block/buzz/rulesets/19321162 --jq '
    .target == "branch" and .enforcement == "active"
    and .current_user_can_bypass == "never"
    and (.conditions.ref_name.include | index("refs/heads/mobile-v*") != null)
    and (.conditions.ref_name.exclude == [])
    and ((has("bypass_actors") | not) or .bypass_actors == [])
    and any(.rules[]; .type == "deletion")
    and any(.rules[]; .type == "non_fast_forward")
    and any(.rules[]; .type == "pull_request"
      and .parameters.required_approving_review_count >= 1
      and .parameters.dismiss_stale_reviews_on_push == true
      and .parameters.require_last_push_approval == true
      and .parameters.required_review_thread_resolution == true)
    and any(.rules[]; .type == "required_status_checks"
      and .parameters.strict_required_status_checks_policy == true
      and ([.parameters.required_status_checks[].context] | index("Mobile") != null)
      and ([.parameters.required_status_checks[].context] | index("DCO Check") != null))
  ')" || { echo "Could not verify mobile stable-branch protection" >&2; return 1; }
  [[ "$valid" == true ]] || {
    echo "Mobile stable-branch ruleset 19321162 must enforce review and checks on mobile-v* without bypass" >&2
    return 1
  }
}

# Compare immutable SHAs, not a mutable branch name. The selected branch may
# advance after this snapshot without changing the authorized candidate source.
require_mobile_release_commit_ancestor() {
  local target_sha="$1" source_sha="$2" merge_base
  merge_base="$(gh api "repos/block/buzz/compare/$target_sha...$source_sha" --jq .merge_base_commit.sha)" || {
    echo "Could not verify candidate source ancestry" >&2
    return 1
  }
  [[ "$merge_base" == "$target_sha" ]] || {
    echo "Candidate commit $target_sha is not in selected source history at $source_sha" >&2
    return 1
  }
}
