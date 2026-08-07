//! Split-capable models — the experimental tier of Share compute.
//!
//! ## What makes a model splittable
//!
//! Not its size, and not its name. A plain GGUF is one opaque blob; splitting
//! needs a *repackaged* model, produced by `mesh-llm models package`, which
//! reads the source tensors, groups them by `layer_index`, and publishes
//! per-layer artifacts alongside a `model-package.json` manifest. mesh-llm
//! detects one by probing for that manifest and explicitly ignores repo naming.
//!
//! The community catalog records the result as a `layer-package` entry, and that
//! flag — never a size heuristic — is what this module reads.
//!
//! ## Why the catalog comes from mesh-llm, not from us
//!
//! `mesh_llm_host_runtime::models::remote_catalog` already loads, caches, and
//! parses <https://huggingface.co/datasets/meshllm/catalog>. Buzz calls that
//! loader instead of fetching and parsing the JSON itself, so there is exactly
//! one implementation of the catalog format. (Hand-parsing it is also a trap:
//! the package discriminator is `type` on the wire but `package_type` in Rust,
//! via `#[serde(rename)]` — a detail a second parser gets wrong silently.)
//!
//! No running node is required, which is the point: you have to choose what to
//! share *before* you start sharing it.
//!
//! ## Why "too large to fit" is the partition, not "multi-machine"
//!
//! A model can carry a layer package and still fit on one machine —
//! `Qwen3-8B-Q4_K_M` is 5 GB and split-capable. mesh-llm's own precedence says
//! the same: `evaluate_model_target_capacity` returns `SingleNodeFit` first and
//! only falls back to `SplitCandidate` when no single node can hold the model.
//!
//! So splittability alone would tell someone a 5 GB model needs a cluster. The
//! experimental tier is the intersection: split-capable **and** beyond this
//! machine's memory. `fits_locally` is reported per entry so that rule lives in
//! one tested place rather than being re-derived per surface.

use serde::Serialize;

use mesh_llm_host_runtime::models::remote_catalog::CatalogEntry;
use mesh_llm_system::hardware;

use super::catalog::{fit_code, ModelFit};

/// A model that can be served across several machines.
#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshSplitEntry {
    /// Model reference, valid as-is in the model field.
    pub name: String,
    /// Catalog size label. Some entries publish `"30 layers"` instead of bytes.
    pub size_label: Option<String>,
    /// Parsed size in GB, or `None` when the catalog publishes no byte size.
    ///
    /// Two upstream entries (gemma-4-26B, Kimi-K2) carry a layer count in the
    /// size field, which parses to nothing. That is also why mesh-llm reports
    /// `unknown_model_size` for them — the size hint is never inserted.
    pub size_gb: Option<f64>,
    /// Layers in the published package: the unit distributed across machines.
    pub layer_count: Option<u32>,
    /// The `-layers` repo holding the package.
    pub package_repo: String,
    /// Whether this machine alone could hold it. See the module note: a
    /// split-capable model that fits locally is not an experimental choice.
    pub fits_locally: bool,
    pub description: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshExperimentalCatalog {
    /// Split-capable models, smallest reach first.
    pub entries: Vec<MeshSplitEntry>,
    /// This machine's usable AI memory, the figure `fits_locally` is against.
    pub vram_gb: f64,
    /// The on-disk catalog cache is older than 24h (or absent and unreachable).
    ///
    /// Surfaced rather than hidden: an empty list means something different when
    /// the catalog could not be read than when it genuinely holds no packages.
    pub stale: bool,
}

/// Load the community catalog and project its split-capable models.
///
/// Blocking: reads the HF cache from disk, and downloads only when nothing is
/// cached. Callers must use `spawn_blocking`.
pub fn experimental_catalog() -> MeshExperimentalCatalog {
    use mesh_llm_host_runtime::models::remote_catalog;

    let survey = hardware::survey();
    let vram_gb = survey.vram_bytes as f64 / 1e9;

    // Disk first: instant, and enough to render. A stale cache still describes
    // real published packages, so it beats blocking the settings screen on a
    // network round trip.
    let _ = remote_catalog::load_catalog_from_disk();
    if remote_catalog::catalog_entries().is_none() {
        // Nothing cached at all — this is the one case worth waiting for.
        let _ = remote_catalog::ensure_catalog();
    }
    let stale = remote_catalog::is_catalog_stale();
    let Some(entries) = remote_catalog::catalog_entries() else {
        return MeshExperimentalCatalog {
            entries: Vec::new(),
            vram_gb,
            stale: true,
        };
    };

    MeshExperimentalCatalog {
        entries: project_split_entries(&entries, vram_gb),
        vram_gb,
        stale,
    }
}

/// Project split-capable models out of catalog entries.
///
/// Split from the I/O above so the selection rules are testable without a
/// hardware survey, a network fetch, or mesh-llm's process-global catalog
/// override (which a parallel test run would race on).
fn project_split_entries(entries: &[CatalogEntry], vram_gb: f64) -> Vec<MeshSplitEntry> {
    let mut projected = Vec::new();
    for entry in entries {
        for (variant_name, variant) in &entry.variants {
            // The layer package is the whole qualification. Never a size
            // heuristic, and never the repo name — mesh-llm ignores naming and
            // probes for the package manifest, so this must agree.
            let Some(package) = variant
                .packages
                .iter()
                .find(|package| package.package_type == "layer-package")
            else {
                continue;
            };
            // `curated.name` is the reference mesh-llm registers as an alias;
            // fall back to the variant key, which it also registers.
            let name = if variant.curated.name.trim().is_empty() {
                variant_name.clone()
            } else {
                variant.curated.name.clone()
            };
            let size_label = variant.curated.size.clone();
            let size_gb = size_label
                .as_deref()
                .map(mesh_llm_client::models::catalog::parse_size_gb)
                .filter(|gb| *gb > 0.0);
            projected.push(MeshSplitEntry {
                name,
                size_label,
                size_gb,
                layer_count: package.layer_count,
                package_repo: package.repo.clone(),
                // Unknown size cannot be claimed to fit. Erring toward "needs
                // several machines" is the honest direction: it offers the split
                // path rather than promising a local start that then fails.
                fits_locally: size_gb
                    .map(|gb| fit_code(gb, vram_gb) != ModelFit::TooLarge)
                    .unwrap_or(false),
                description: variant.curated.description.clone(),
            });
        }
    }

    projected.sort_by(|left, right| {
        // Smallest reach first: the least ambitious split is the likeliest to
        // actually form. Unknown sizes sort last — they promise nothing.
        let key = |entry: &MeshSplitEntry| entry.size_gb.unwrap_or(f64::MAX);
        key(left)
            .partial_cmp(&key(right))
            .unwrap_or(std::cmp::Ordering::Equal)
            .then_with(|| left.name.cmp(&right.name))
    });
    projected.dedup_by(|left, right| left.name == right.name);
    projected
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Fixtures are JSON, not constructed structs.
    ///
    /// Partly forced — mesh-llm re-exports `CatalogVariant` and friends only
    /// under its own `#[cfg(test)]`, so a downstream crate cannot build them —
    /// but better regardless: the wire format is where this projection can
    /// actually go wrong. The package discriminator is `type` in JSON and
    /// `package_type` in Rust (via `#[serde(rename)]`), and an earlier revision
    /// of this work read the Rust name off the wire and concluded that *zero*
    /// models were split-capable. A struct-built fixture would have passed that
    /// bug straight through; this one cannot.
    fn entries(json: &str) -> Vec<CatalogEntry> {
        vec![serde_json::from_str(json).expect("catalog entry fixture")]
    }

    fn one_variant(name: &str, size: &str, packages: &str) -> Vec<CatalogEntry> {
        entries(&format!(
            r#"{{"schema_version":1,"source_repo":"vendor/Model-GGUF","variants":{{
                 "{name}":{{
                   "source":{{"repo":"vendor/Model-GGUF"}},
                   "curated":{{"name":"{name}","size":"{size}"}},
                   "packages":{packages}
                 }}}}}}"#
        ))
    }

    const LAYER_PKG: &str =
        r#"[{"type":"layer-package","repo":"meshllm/Model-layers","layer_count":64}]"#;

    #[test]
    fn only_layer_packaged_variants_qualify() {
        // A plain GGUF is one opaque blob; splitting needs the repackaged form,
        // and size has no bearing on the question. mesh-llm probes for the
        // package manifest and ignores repo naming, so this must agree.
        let json = format!(
            r#"{{"schema_version":1,"source_repo":"vendor/M-GGUF","variants":{{
                 "Plain-Q4":{{"source":{{"repo":"vendor/M-GGUF"}},
                   "curated":{{"name":"Plain-Q4","size":"400GB"}},"packages":[]}},
                 "Split-Q4":{{"source":{{"repo":"vendor/M-GGUF"}},
                   "curated":{{"name":"Split-Q4","size":"20GB"}},"packages":{LAYER_PKG}}}
               }}}}"#
        );
        let projected = project_split_entries(&entries(&json), 100.0);
        assert_eq!(projected.len(), 1);
        assert_eq!(projected[0].name, "Split-Q4");
        assert_eq!(projected[0].layer_count, Some(64));
        assert_eq!(projected[0].package_repo, "meshllm/Model-layers");
    }

    #[test]
    fn a_split_capable_model_can_still_fit_locally() {
        // Qwen3-8B is 5GB and split-capable. Calling it "needs several machines"
        // would be false, and mesh-llm agrees: `evaluate_model_target_capacity`
        // returns SingleNodeFit before it ever considers a split. This is why
        // the experimental tier gates on fit, not on splittability.
        let fixture = one_variant("Small-Q4", "5.0GB", LAYER_PKG);
        assert!(project_split_entries(&fixture, 100.0)[0].fits_locally);
        assert!(!project_split_entries(&fixture, 4.0)[0].fits_locally);
    }

    #[test]
    fn a_layer_count_in_the_size_field_is_not_a_size() {
        // Two upstream entries publish "30 layers" where bytes belong. It parses
        // to nothing -- which is also why mesh-llm reports `unknown_model_size`
        // for the very model this machine serves.
        let projected = project_split_entries(&one_variant("L-Q4", "30 layers", LAYER_PKG), 512.0);
        assert_eq!(projected[0].size_gb, None);
        assert_eq!(projected[0].size_label.as_deref(), Some("30 layers"));
        // Unknown size is never claimed to fit, even on a huge machine: offering
        // the split path is recoverable, promising a local start is not.
        assert!(!projected[0].fits_locally);
    }

    #[test]
    fn smallest_reach_sorts_first_and_unknown_sizes_last() {
        let json = format!(
            r#"{{"schema_version":1,"source_repo":"v/M","variants":{{
                 "Big":{{"source":{{"repo":"v/M"}},"curated":{{"name":"Big","size":"300GB"}},"packages":{LAYER_PKG}}},
                 "Small":{{"source":{{"repo":"v/M"}},"curated":{{"name":"Small","size":"20GB"}},"packages":{LAYER_PKG}}},
                 "Unsized":{{"source":{{"repo":"v/M"}},"curated":{{"name":"Unsized","size":"61 layers"}},"packages":{LAYER_PKG}}}
               }}}}"#
        );
        let names: Vec<_> = project_split_entries(&entries(&json), 100.0)
            .into_iter()
            .map(|entry| entry.name)
            .collect();
        assert_eq!(names, vec!["Small", "Big", "Unsized"]);
    }

    #[test]
    fn the_wire_discriminator_is_type_not_package_type() {
        // Regression guard for the mistake that produced "0 split-capable
        // models": reading Rust's field name (`package_type`) off a wire that
        // spells it `type`.
        //
        // The guard turned out stronger than expected. A wrong key does not
        // project to an empty list -- serde refuses the entry outright, because
        // `type` is required and unknown fields are ignored. So a format drift
        // upstream surfaces as a parse error, never as a silently empty
        // experimental tier, which is the failure mode that would have been
        // impossible to notice.
        let wrong = r#"{"schema_version":1,"source_repo":"v/M","variants":{
             "M-Q4":{"source":{"repo":"v/M"},"curated":{"name":"M-Q4","size":"20GB"},
               "packages":[{"package_type":"layer-package","repo":"meshllm/M-layers"}]}}}"#;
        let parsed = serde_json::from_str::<CatalogEntry>(wrong);
        assert!(parsed.is_err(), "wrong discriminator must not parse");

        // And the correct spelling projects one entry.
        assert_eq!(
            project_split_entries(&one_variant("M-Q4", "20GB", LAYER_PKG), 100.0).len(),
            1
        );
    }
}
