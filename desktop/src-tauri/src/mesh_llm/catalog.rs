//! Hardware-aware model catalog for the Share-compute picker.
//!
//! Same diagnose pattern as mesh-console: survey the machine's AI memory,
//! rank mesh-llm's curated `MODEL_CATALOG` by how each model fits, mark what
//! is already in the HuggingFace cache, and recommend a best fit. This
//! replaces guessing into a free-text model field.

use serde::Serialize;

use mesh_llm_client::models::catalog::{parse_size_gb, MODEL_CATALOG};
use mesh_llm_client::network::nostr::auto_model_pack;
use mesh_llm_node::models::{default_huggingface_cache_dir, scan_installed_models};
use mesh_llm_system::hardware;
use mesh_llm_system::vram::format_rated_capacity;

/// How a model sits inside this machine's usable AI memory.
/// Mirrors mesh-llm's private `fit_code_for_size_label` thresholds.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum ModelFit {
    Comfortable,
    Tight,
    Tradeoff,
    TooLarge,
}

fn fit_code(model_gb: f64, vram_gb: f64) -> ModelFit {
    if model_gb <= vram_gb * 0.6 {
        ModelFit::Comfortable
    } else if model_gb <= vram_gb * 0.9 {
        ModelFit::Tight
    } else if model_gb <= vram_gb * 1.1 {
        ModelFit::Tradeoff
    } else {
        ModelFit::TooLarge
    }
}

fn fit_rank(fit: ModelFit) -> u8 {
    match fit {
        ModelFit::Comfortable => 0,
        ModelFit::Tight => 1,
        ModelFit::Tradeoff => 2,
        ModelFit::TooLarge => 3,
    }
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshCatalogEntry {
    /// Catalog name — what the user serves (goes straight into the model field).
    pub name: String,
    /// Display size, e.g. "5.0GB".
    pub size: String,
    pub size_gb: f64,
    pub description: String,
    pub fit: ModelFit,
    pub installed: bool,
    pub recommended: bool,
    /// A "shared" model is a layer-package (`meshllm/…-layers`) that is too big
    /// for any single machine and runs split across several members. When true,
    /// serving it does nothing until enough members join and host it together;
    /// mesh-llm then auto-splits it across the group.
    pub shared: bool,
    /// For shared models: rough number of members needed to host this model on
    /// machines like this one. Advisory only — the mesh decides the real split.
    pub estimated_members: Option<u32>,
}

/// A curated shared (split) model — a `meshllm/…-layers` layer package that is
/// too large for a single node and is served split across several members.
///
/// These are intentionally hand-picked (not the full ~79-entry generated
/// catalog) so the Advanced → "Join a shared model" list stays legible: one
/// solid pick per size tier, from a 2-machine demo up to a genuine
/// "no single box could ever fit this" showcase. Refs are the canonical
/// `meshllm/*-layers` HuggingFace repos.
struct SharedModel {
    /// Layer-package ref served verbatim (e.g. `meshllm/Qwen3-235B-A22B-UD-Q4_K_XL-layers`).
    name: &'static str,
    /// Display size of the full model, e.g. "134GB".
    size: &'static str,
    /// One-line human description for the picker row.
    description: &'static str,
}

/// Curated shared/split models, smallest → largest. Sizes are the full
/// (unsplit) model footprint, which is what determines how many members are
/// needed. `estimated_members` is derived per-machine at build time.
const SHARED_MODELS: &[SharedModel] = &[
    SharedModel {
        name: "meshllm/Qwen3-8B-Q4_K_M-layers",
        size: "5.0GB",
        description: "Small demo split — runs across two machines. Good for trying shared compute.",
    },
    SharedModel {
        name: "meshllm/Qwen3-32B-UD-Q4_K_XL-layers",
        size: "20GB",
        description: "Mid-size model split across a few members.",
    },
    SharedModel {
        name: "meshllm/Llama-3.3-70B-Instruct-Q3_K_M-layers",
        size: "34GB",
        description: "70B-class general model, hosted by the group.",
    },
    SharedModel {
        name: "meshllm/gpt-oss-120b-UD-Q4_K_XL-layers",
        size: "65GB",
        description: "120B open model — needs several members together.",
    },
    SharedModel {
        name: "meshllm/Qwen3-235B-A22B-UD-Q4_K_XL-layers",
        size: "134GB",
        description: "Large 235B model. Too big for one machine — the group hosts it.",
    },
    SharedModel {
        name: "meshllm/Qwen3-Coder-480B-A35B-Instruct-UD-Q4_K_XL-layers",
        size: "294GB",
        description: "480B coding model. A serious group effort across many members.",
    },
    SharedModel {
        name: "meshllm/DeepSeek-V3.2-UD-Q4_K_XL-layers",
        size: "382GB",
        description: "Frontier 671B-class model. Hosted only by a large group together.",
    },
];

/// Estimate how many members like this machine are needed to host `model_gb`,
/// leaving realistic per-node headroom. Conservative: assumes each member
/// contributes at most ~80% of usable AI memory to its slice, and always
/// needs at least two participants to form a split. `None` when hardware
/// capacity is unknown.
fn estimate_members(model_gb: f64, vram_gb: f64) -> Option<u32> {
    if vram_gb <= 0.0 {
        return None;
    }
    let per_node = vram_gb * 0.8;
    if per_node <= 0.0 {
        return Some(2);
    }
    let needed = (model_gb / per_node).ceil() as u32;
    Some(needed.max(2))
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MeshModelCatalog {
    /// e.g. "Apple M3 Max"
    pub gpu_name: Option<String>,
    /// Usable AI memory, display-formatted (e.g. "96 GB").
    pub vram_display: String,
    pub vram_gb: f64,
    /// Best-fit catalog name for this hardware, if any.
    pub recommended: Option<String>,
    /// Ranked: recommended first, then by fit, then larger first within a fit.
    /// These are single-machine ("solo") models.
    pub entries: Vec<MeshCatalogEntry>,
    /// Curated shared/split models (`meshllm/…-layers`), smallest → largest.
    /// Each is too big for one machine and runs split across several members.
    pub shared: Vec<MeshCatalogEntry>,
}

/// Survey hardware and rank the curated catalog for this machine.
/// Draft (speculative-decoding) models are excluded — they are not something
/// a person shares directly.
pub fn model_catalog() -> MeshModelCatalog {
    let survey = hardware::survey();
    let vram_gb = survey.vram_bytes as f64 / 1e9;
    build_catalog(
        survey.gpu_name.clone(),
        survey.vram_bytes,
        vram_gb,
        &installed_names(),
    )
}

fn installed_names() -> Vec<(String, String)> {
    let cache = default_huggingface_cache_dir();
    scan_installed_models(cache)
        .into_iter()
        .map(|m| {
            let file = m
                .path
                .file_name()
                .and_then(|f| f.to_str())
                .unwrap_or_default()
                .to_string();
            (file, m.model_ref)
        })
        .collect()
}

fn build_catalog(
    gpu_name: Option<String>,
    vram_bytes: u64,
    vram_gb: f64,
    installed: &[(String, String)],
) -> MeshModelCatalog {
    let is_installed = |file: &str, name: &str| {
        installed
            .iter()
            .any(|(f, model_ref)| f == file || model_ref.contains(name))
    };
    let mut entries: Vec<MeshCatalogEntry> = MODEL_CATALOG
        .iter()
        .filter(|m| !is_draft_only(&m.name))
        .map(|m| {
            let size_gb = parse_size_gb(&m.size);
            MeshCatalogEntry {
                fit: fit_code(size_gb, vram_gb),
                installed: is_installed(&m.file, &m.name),
                recommended: false,
                name: m.name.clone(),
                size: m.size.clone(),
                size_gb,
                description: m.description.clone(),
                shared: false,
                estimated_members: None,
            }
        })
        .collect();

    let recommended = auto_model_pack(vram_gb).into_iter().next();
    for entry in &mut entries {
        entry.recommended = recommended.as_deref() == Some(entry.name.as_str());
    }

    entries.sort_by(|a, b| {
        b.recommended
            .cmp(&a.recommended)
            .then(fit_rank(a.fit).cmp(&fit_rank(b.fit)))
            .then(b.size_gb.total_cmp(&a.size_gb))
    });

    // Shared/split models: curated layer packages, presented smallest → largest.
    // Every one is `fit: TooLarge` for a single node by definition (that is the
    // whole point) — the "fit" story for these is "how many members", carried by
    // `estimated_members`, not the solo fit code.
    let shared: Vec<MeshCatalogEntry> = SHARED_MODELS
        .iter()
        .map(|m| {
            let size_gb = parse_size_gb(m.size);
            MeshCatalogEntry {
                fit: ModelFit::TooLarge,
                installed: false,
                recommended: false,
                name: m.name.to_string(),
                size: m.size.to_string(),
                size_gb,
                description: m.description.to_string(),
                shared: true,
                estimated_members: estimate_members(size_gb, vram_gb),
            }
        })
        .collect();

    MeshModelCatalog {
        gpu_name,
        vram_display: format_rated_capacity(vram_bytes),
        vram_gb,
        recommended,
        entries,
        shared,
    }
}

/// A model that exists in the catalog only as another model's draft
/// (speculative decoding helper) — identified by being referenced in any
/// `draft` field. People share chat models, not drafts.
fn is_draft_only(name: &str) -> bool {
    MODEL_CATALOG
        .iter()
        .any(|m| m.draft.as_deref() == Some(name))
        && !MODEL_CATALOG
            .iter()
            .any(|m| m.name == name && m.draft.is_some())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fit_thresholds_match_mesh_llm() {
        // 10GB model on various machines. Thresholds are 0.6 / 0.9 / 1.1.
        assert_eq!(fit_code(10.0, 20.0), ModelFit::Comfortable);
        assert_eq!(fit_code(10.0, 12.0), ModelFit::Tight);
        assert_eq!(fit_code(10.0, 10.0), ModelFit::Tradeoff);
        assert_eq!(fit_code(10.0, 8.0), ModelFit::TooLarge);
    }

    #[test]
    fn catalog_ranks_recommended_first_then_fit() {
        let catalog = build_catalog(Some("Test GPU".into()), 24_000_000_000, 24.0, &[]);
        assert!(
            !catalog.entries.is_empty(),
            "curated catalog must not be empty"
        );
        // The recommended entry (if present in the catalog) must be first.
        if let Some(recommended) = &catalog.recommended {
            if catalog.entries.iter().any(|e| &e.name == recommended) {
                assert_eq!(&catalog.entries[0].name, recommended);
                assert!(catalog.entries[0].recommended);
            }
        }
        // Fit ranks must be non-decreasing after the recommended head.
        let ranks: Vec<u8> = catalog
            .entries
            .iter()
            .skip_while(|e| e.recommended)
            .map(|e| fit_rank(e.fit))
            .collect();
        assert!(
            ranks.windows(2).all(|w| w[0] <= w[1]),
            "fit ranks out of order: {ranks:?}"
        );
    }

    #[test]
    fn recommendation_uses_mesh_llm_auto_selection() {
        let catalog = build_catalog(None, 62_000_000_000, 62.0, &[]);
        assert_eq!(
            catalog.recommended,
            auto_model_pack(62.0).into_iter().next()
        );
    }

    #[test]
    fn shared_models_are_present_ranked_and_marked() {
        let catalog = build_catalog(Some("Test GPU".into()), 24_000_000_000, 24.0, &[]);
        assert!(
            !catalog.shared.is_empty(),
            "curated shared catalog must not be empty"
        );
        // Every shared entry is a layer package and flagged shared + TooLarge.
        for entry in &catalog.shared {
            assert!(
                entry.shared,
                "shared entry must have shared=true: {entry:?}"
            );
            assert!(
                entry.name.contains("-layers"),
                "shared entry must be a layer package: {}",
                entry.name
            );
            assert_eq!(
                entry.fit,
                ModelFit::TooLarge,
                "shared entries are too large for one node by definition"
            );
        }
        // Presented smallest → largest.
        let sizes: Vec<f64> = catalog.shared.iter().map(|e| e.size_gb).collect();
        assert!(
            sizes.windows(2).all(|w| w[0] <= w[1]),
            "shared models must be ordered smallest → largest: {sizes:?}"
        );
    }

    #[test]
    fn estimate_members_needs_at_least_two_and_scales_with_size() {
        // Tiny model on a big machine still needs the two-participant minimum.
        assert_eq!(estimate_members(5.0, 96.0), Some(2));
        // A model needing more than one node's ~80% share needs more members.
        // 134GB / (24 * 0.8 = 19.2) = 6.98 -> 7 members.
        assert_eq!(estimate_members(134.0, 24.0), Some(7));
        // Unknown capacity yields no estimate.
        assert_eq!(estimate_members(134.0, 0.0), None);
    }

    #[test]
    fn solo_entries_are_not_shared() {
        let catalog = build_catalog(None, 96_000_000_000, 96.0, &[]);
        assert!(
            catalog.entries.iter().all(|e| !e.shared),
            "solo catalog entries must never be marked shared"
        );
    }

    #[test]
    fn installed_matches_by_file_or_model_ref() {
        let installed = vec![(
            "Qwen3-8B-Q4_K_M.gguf".to_string(),
            "unsloth/Qwen3-8B-GGUF:Q4_K_M".to_string(),
        )];
        let catalog = build_catalog(None, 96_000_000_000, 96.0, &installed);
        let qwen8b = catalog.entries.iter().find(|e| e.name == "Qwen3-8B-Q4_K_M");
        if let Some(entry) = qwen8b {
            assert!(entry.installed, "cached file must mark entry installed");
        }
        // A machine with nothing installed marks nothing installed.
        let empty = build_catalog(None, 96_000_000_000, 96.0, &[]);
        assert!(empty.entries.iter().all(|e| !e.installed));
    }
}
