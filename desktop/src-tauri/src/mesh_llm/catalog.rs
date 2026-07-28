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
    /// Buzz-curated pick — known to survive the agent harness. Curated
    /// entries render above the fold; everything else is "advanced".
    pub curated: bool,
    /// Whether this model's weights plus working headroom actually fit in the
    /// free space on the model-cache volume. A model can fit in AI memory and
    /// still be undownloadable; without this the failure only surfaces at the
    /// end of a multi-gigabyte download. `true` when the probe failed, so a
    /// bad probe never blocks a download.
    pub fits_disk: bool,
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
    /// Free space on the model-cache volume. Zero when the probe failed, in
    /// which case the UI shows "—" rather than a wrong number.
    pub disk_free_bytes: u64,
    pub disk_free_display: String,
}

/// Working headroom multiplier over raw weight size: downloads land in a
/// temp file alongside the final blob, so a model needs more than its own
/// size free to complete.
const DISK_HEADROOM: f64 = 1.15;

/// Whether `size_gb` of weights fit in `disk_free_bytes`. A failed probe
/// (`0`) returns `true` so a bad probe never blocks a download.
fn fits_disk(size_gb: f64, disk_free_bytes: u64) -> bool {
    if disk_free_bytes == 0 {
        return true;
    }
    size_gb * DISK_HEADROOM <= disk_free_bytes as f64 / 1e9
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
        free_disk_bytes(&default_huggingface_cache_dir()),
    )
}

/// Free bytes on the volume backing `path`, via POSIX `df`. Returns 0 on any
/// exec/parse failure so callers fall back to "—" instead of a wrong figure.
fn free_disk_bytes(path: &std::path::Path) -> u64 {
    // `df` needs an existing path; walk up to the nearest ancestor that
    // exists (the HF cache dir is absent until the first download).
    let mut probe = path;
    while !probe.exists() {
        match probe.parent() {
            Some(parent) => probe = parent,
            None => return 0,
        }
    }
    let output = match std::process::Command::new("df")
        .args(["-k", "-P"])
        .arg(probe)
        .output()
    {
        Ok(output) if output.status.success() => output,
        _ => return 0,
    };
    // `df -P` prints a header then one data line; column 4 is available 1K blocks.
    String::from_utf8_lossy(&output.stdout)
        .lines()
        .nth(1)
        .and_then(|line| line.split_whitespace().nth(3))
        .and_then(|kb| kb.parse::<u64>().ok())
        .map(|kb| kb * 1024)
        .unwrap_or(0)
}

fn format_disk(bytes: u64) -> String {
    if bytes == 0 {
        return "—".to_string();
    }
    let gb = bytes as f64 / 1e9;
    if gb >= 1000.0 {
        format!("{:.1} TB", gb / 1000.0)
    } else if gb >= 10.0 {
        format!("{} GB", gb.round() as u64)
    } else {
        format!("{gb:.1} GB")
    }
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
    disk_free_bytes: u64,
) -> MeshModelCatalog {
    let is_installed = |file: &str, name: &str| {
        installed
            .iter()
            .any(|(f, model_ref)| f == file || model_ref.contains(name))
    };
    let recommended = auto_model_pack(vram_gb).into_iter().next();
    let mut entries: Vec<MeshCatalogEntry> = MODEL_CATALOG
        .iter()
        .filter(|m| !is_draft_only(&m.name))
        .map(|m| {
            let size_gb = parse_size_gb(&m.size);
            MeshCatalogEntry {
                fit: fit_code(size_gb, vram_gb),
                installed: is_installed(&m.file, &m.name),
                recommended: recommended.as_deref() == Some(m.name.as_str()),
                curated: recommended.as_deref() == Some(m.name.as_str()),
                fits_disk: fits_disk(size_gb, disk_free_bytes),
                name: m.name.clone(),
                size: m.size.clone(),
                size_gb,
                description: m.description.clone(),
            }
        })
        .collect();

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
        disk_free_bytes,
        disk_free_display: format_disk(disk_free_bytes),
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
        let catalog = build_catalog(Some("Test GPU".into()), 24_000_000_000, 24.0, &[], 0);
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
    fn recommendation_matches_desktop_app_model_packs() {
        let big = build_catalog(None, 128_000_000_000, 128.0, &[], 0);
        assert_eq!(big.recommended.as_deref(), Some("Qwen3-Coder-Next-Q4_K_M"));
        let medium = build_catalog(None, 56_000_000_000, 56.0, &[], 0);
        assert_eq!(medium.recommended.as_deref(), Some("GLM-4.7-Flash-Q4_K_M"));
        let tiny = build_catalog(None, 4_000_000_000, 4.0, &[], 0);
        assert_eq!(tiny.recommended.as_deref(), Some("Qwen3-4B-Q4_K_M"));
    }

    #[test]
    fn recommended_model_leads_the_catalog() {
        let catalog = build_catalog(None, 128_000_000_000, 128.0, &[], 0);
        assert_eq!(catalog.entries[0].name, "Qwen3-Coder-Next-Q4_K_M");
        assert!(catalog.entries[0].recommended && catalog.entries[0].curated);
        assert!(catalog.entries[1..].iter().all(|e| !e.curated));
    }

    #[test]
    fn installed_matches_by_file_or_model_ref() {
        let installed = vec![(
            "Qwen3-8B-Q4_K_M.gguf".to_string(),
            "unsloth/Qwen3-8B-GGUF:Q4_K_M".to_string(),
        )];
        let catalog = build_catalog(None, 96_000_000_000, 96.0, &installed, 0);
        let qwen8b = catalog.entries.iter().find(|e| e.name == "Qwen3-8B-Q4_K_M");
        if let Some(entry) = qwen8b {
            assert!(entry.installed, "cached file must mark entry installed");
        }
        // A machine with nothing installed marks nothing installed.
        let empty = build_catalog(None, 96_000_000_000, 96.0, &[], 0);
        assert!(empty.entries.iter().all(|e| !e.installed));
    }

    #[test]
    fn disk_probe_gates_entries_and_never_blocks_on_failure() {
        // 19GB free cannot hold the 48GB recommendation plus headroom.
        let tight = build_catalog(None, 128_000_000_000, 128.0, &[], 19_000_000_000);
        let large = tight
            .entries
            .iter()
            .find(|e| e.name == "Qwen3-Coder-Next-Q4_K_M")
            .expect("desktop-app recommendation present");
        assert!(!large.fits_disk, "48GB weights must not fit in 19GB free");
        assert_eq!(tight.disk_free_bytes, 19_000_000_000);
        assert_eq!(tight.disk_free_display, "19 GB");

        // Plenty of room: same entry fits.
        let roomy = build_catalog(None, 128_000_000_000, 128.0, &[], 2_000_000_000_000);
        assert!(
            roomy
                .entries
                .iter()
                .find(|e| e.name == "Qwen3-Coder-Next-Q4_K_M")
                .expect("desktop-app recommendation present")
                .fits_disk
        );
        assert_eq!(roomy.disk_free_display, "2.0 TB");

        // Failed probe (0) must never mark anything unfittable.
        let unknown = build_catalog(None, 128_000_000_000, 128.0, &[], 0);
        assert!(unknown.entries.iter().all(|e| e.fits_disk));
        assert_eq!(unknown.disk_free_display, "\u{2014}");
    }

    #[test]
    fn format_disk_scales_and_falls_back() {
        assert_eq!(format_disk(0), "\u{2014}");
        assert_eq!(format_disk(512 * 1_000_000_000), "512 GB");
        assert_eq!(format_disk(2_500 * 1_000_000_000), "2.5 TB");
        assert_eq!(format_disk(4_400_000_000), "4.4 GB");
    }

    #[test]
    fn real_disk_probe_returns_a_figure() {
        assert!(free_disk_bytes(std::path::Path::new("/")) > 0);
    }
}
