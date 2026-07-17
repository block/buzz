//! Hardware-aware model catalog for the Share-compute picker.
//!
//! Same diagnose pattern as mesh-console: survey the machine's AI memory,
//! rank mesh-llm's curated `MODEL_CATALOG` by how each model fits, mark what
//! is already in the HuggingFace cache, and recommend a best fit. This
//! replaces guessing into a free-text model field.

use serde::Serialize;

use mesh_llm_client::models::catalog::{parse_size_gb, MODEL_CATALOG};
use mesh_llm_node::models::{default_huggingface_cache_dir, scan_installed_models};
use mesh_llm_system::hardware;
use mesh_llm_system::vram::{format_rated_capacity, rated_capacity_gb};

/// Buzz-curated tier picks. These are the models we know survive the agent
/// harness on shared compute — deliberately non-reasoning instruction models,
/// so agents stay snappy instead of burning hidden reasoning tokens.
///
/// The large pick is resolved through mesh-llm's remote catalog
/// (huggingface.co/datasets/meshllm/catalog), so it does not need to exist in
/// the compiled `MODEL_CATALOG`; the entry is synthesized below.
const CURATED_LARGE: &str = "gemma-4-26B-A4B-it-UD-Q4_K_M";
const CURATED_LARGE_SIZE: &str = "17GB";
const CURATED_LARGE_FILE: &str = "gemma-4-26B-A4B-it-UD-Q4_K_M.gguf";
const CURATED_LARGE_DESCRIPTION: &str =
    "Gemma 4 26B MoE (4B active) — Buzz default for 64GB+ machines";
const CURATED_SMALL: &str = "Gemma-4-E4B-it-Q4_K_M";
/// Rated-capacity boundary between the two curated tiers, in GB (marketing
/// capacity — a "64GB" Mac rates as 64 even though usable AI memory is less).
const CURATED_LARGE_MIN_RATED_GB: u64 = 64;

/// The Buzz-curated recommendation for a machine's rated memory capacity.
fn buzz_recommended_model(rated_gb: Option<u64>) -> &'static str {
    match rated_gb {
        Some(gb) if gb >= CURATED_LARGE_MIN_RATED_GB => CURATED_LARGE,
        _ => CURATED_SMALL,
    }
}

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
    /// Buzz-curated pick — known to survive the agent harness. Curated
    /// entries render above the fold; everything else is "advanced".
    pub curated: bool,
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
    pub entries: Vec<MeshCatalogEntry>,
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
                curated: false,
                name: m.name.clone(),
                size: m.size.clone(),
                size_gb,
                description: m.description.clone(),
            }
        })
        .collect();

    // The compiled MODEL_CATALOG does not know the Buzz large pick; it
    // resolves through mesh-llm's remote catalog at download time. Synthesize
    // its entry so the picker can offer it.
    if !entries.iter().any(|e| e.name == CURATED_LARGE) {
        let size_gb = parse_size_gb(CURATED_LARGE_SIZE);
        entries.push(MeshCatalogEntry {
            fit: fit_code(size_gb, vram_gb),
            installed: is_installed(CURATED_LARGE_FILE, CURATED_LARGE),
            recommended: false,
            curated: false,
            name: CURATED_LARGE.to_string(),
            size: CURATED_LARGE_SIZE.to_string(),
            size_gb,
            description: CURATED_LARGE_DESCRIPTION.to_string(),
        });
    }

    let recommended = Some(buzz_recommended_model(rated_capacity_gb(vram_bytes)).to_string());
    for entry in &mut entries {
        entry.recommended = recommended.as_deref() == Some(entry.name.as_str());
        // Both curated tiers are always offered: the recommended one for this
        // machine plus the other pick (e.g. the small one as an explicit
        // lighter choice on big machines).
        entry.curated = entry.name == CURATED_LARGE || entry.name == CURATED_SMALL;
    }

    entries.sort_by(|a, b| {
        b.recommended
            .cmp(&a.recommended)
            .then(b.curated.cmp(&a.curated))
            .then(fit_rank(a.fit).cmp(&fit_rank(b.fit)))
            .then(b.size_gb.total_cmp(&a.size_gb))
    });

    MeshModelCatalog {
        gpu_name,
        vram_display: format_rated_capacity(vram_bytes),
        vram_gb,
        recommended,
        entries,
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
        // Fit ranks must be non-decreasing after the recommended/curated head.
        let ranks: Vec<u8> = catalog
            .entries
            .iter()
            .skip_while(|e| e.recommended || e.curated)
            .map(|e| fit_rank(e.fit))
            .collect();
        assert!(
            ranks.windows(2).all(|w| w[0] <= w[1]),
            "fit ranks out of order: {ranks:?}"
        );
    }

    #[test]
    fn recommendation_follows_buzz_curated_tiers() {
        // 64GB+ rated machines get the large curated pick.
        let large = build_catalog(None, 64_000_000_000, 64.0, &[]);
        assert_eq!(large.recommended.as_deref(), Some(CURATED_LARGE));
        let big = build_catalog(None, 128_000_000_000, 128.0, &[]);
        assert_eq!(big.recommended.as_deref(), Some(CURATED_LARGE));
        // Below the boundary: the small curated pick — never a reasoning
        // model, never sub-4B guesswork.
        let small = build_catalog(None, 32_000_000_000, 32.0, &[]);
        assert_eq!(small.recommended.as_deref(), Some(CURATED_SMALL));
        let tiny = build_catalog(None, 16_000_000_000, 16.0, &[]);
        assert_eq!(tiny.recommended.as_deref(), Some(CURATED_SMALL));
    }

    #[test]
    fn curated_picks_lead_the_catalog() {
        let catalog = build_catalog(None, 96_000_000_000, 96.0, &[]);
        // Recommended curated entry first, the other curated pick second,
        // advanced entries after.
        assert_eq!(catalog.entries[0].name, CURATED_LARGE);
        assert!(catalog.entries[0].recommended && catalog.entries[0].curated);
        assert_eq!(catalog.entries[1].name, CURATED_SMALL);
        assert!(catalog.entries[1].curated && !catalog.entries[1].recommended);
        assert!(catalog.entries[2..].iter().all(|e| !e.curated));
        // The synthesized large pick carries a real size for fit ranking.
        assert!(catalog.entries[0].size_gb > 10.0);
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
