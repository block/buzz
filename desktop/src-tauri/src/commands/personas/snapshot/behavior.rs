use crate::managed_agents::{
    resolve_mint_behavioral_defaults, validate_respond_to_allowlist, ReplyPlacement, RespondTo,
};

/// Resolve the behavioral defaults for an incoming agent snapshot.
///
/// This is the single authoritative selection path for import-time allowlist
/// and behavioral decisions. The Keep/Clear toggle is shown whenever the raw
/// allowlist is non-empty, regardless of the source mode.
#[cfg(test)]
pub(crate) fn resolve_snapshot_import_behavior(
    raw_respond_to: Option<&str>,
    raw_allowlist: &[String],
    parallelism: Option<u32>,
    keep_allowlist: bool,
) -> Result<crate::managed_agents::MintBehavioralDefaults, String> {
    resolve_snapshot_import_behavior_with_reply(
        raw_respond_to,
        raw_allowlist,
        parallelism,
        None,
        keep_allowlist,
    )
}

pub(crate) fn resolve_snapshot_import_behavior_with_reply(
    raw_respond_to: Option<&str>,
    raw_allowlist: &[String],
    parallelism: Option<u32>,
    raw_reply_placement: Option<&str>,
    keep_allowlist: bool,
) -> Result<crate::managed_agents::MintBehavioralDefaults, String> {
    let normalized_allowlist = validate_respond_to_allowlist(raw_allowlist)?;
    let source_mode = raw_respond_to.map(RespondTo::parse_wire).transpose()?;
    let is_source_allowlist_mode = source_mode == Some(RespondTo::Allowlist);
    let has_source_allowlist = !normalized_allowlist.is_empty();

    if is_source_allowlist_mode && !has_source_allowlist {
        return Err(
            "snapshot respond-to mode is 'allowlist' but the allowlist is empty — \
             cannot import: no pubkeys to grant access to"
                .to_string(),
        );
    }

    let (resolved_mode, resolved_allowlist) = if has_source_allowlist {
        if keep_allowlist {
            (source_mode, normalized_allowlist)
        } else if is_source_allowlist_mode {
            (Some(RespondTo::OwnerOnly), Vec::new())
        } else {
            (source_mode, Vec::new())
        }
    } else {
        (source_mode, normalized_allowlist)
    };

    let reply_placement = raw_reply_placement
        .map(ReplyPlacement::parse_wire)
        .transpose()?;

    resolve_mint_behavioral_defaults(
        resolved_mode,
        resolved_allowlist,
        parallelism,
        reply_placement,
        None,
        None,
    )
}
