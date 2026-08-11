//! Hardware-aware model catalog for the Share-compute picker.
//!
//! Same diagnose pattern as mesh-console: survey the machine's AI memory,
//! rank mesh-llm's curated `MODEL_CATALOG` by how each model fits, mark what
//! is already in the HuggingFace cache, and recommend a best fit. This
//! replaces guessing into a free-text model field.

use serde::Serialize;

use mesh_llm_client::models::catalog::{parse_size_gb, MODEL_CATALOG};
use mesh_llm_node::models::{default_huggingface_cache_dir, scan_installed_models};
use mesh_llm_system::vram::{format_rated_capacity, rated_capacity_gb};

#[cfg(not(target_os = "windows"))]
use mesh_llm_system::hardware;

/// Buzz-curated tier picks. These are the models we know survive the agent
/// harness on shared compute — deliberately non-reasoning instruction models,
/// so agents stay snappy instead of burning hidden reasoning tokens.
///
/// The large pick is resolved through mesh-llm's remote catalog
/// (huggingface.co/datasets/meshllm/catalog), so it does not need to exist in
/// the compiled `MODEL_CATALOG`; the entry is synthesized below.
const CURATED_LARGE: &str = "unsloth/gemma-4-26B-A4B-it-GGUF:UD-Q4_K_M";
const CURATED_LARGE_ALIAS: &str = "gemma-4-26B-A4B-it-UD-Q4_K_M";
const CURATED_LARGE_SIZE: &str = "17GB";
const CURATED_LARGE_FILE: &str = "gemma-4-26B-A4B-it-UD-Q4_K_M.gguf";
const CURATED_LARGE_DESCRIPTION: &str =
    "Gemma 4 26B MoE (4B active) — Buzz default for 64GB+ machines";
const CURATED_SMALL: &str = "unsloth/gemma-4-E4B-it-GGUF:Q4_K_M";
const CURATED_SMALL_ALIAS: &str = "Gemma-4-E4B-it-Q4_K_M";
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

/// Convert Buzz's pre-0.74 curated package aliases into the canonical model
/// ids advertised and accepted by Mesh's OpenAI ingress.
pub(crate) fn canonical_curated_model_id(model_id: &str) -> &str {
    match model_id.trim() {
        CURATED_SMALL_ALIAS => CURATED_SMALL,
        CURATED_LARGE_ALIAS => CURATED_LARGE,
        other => other,
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
    Unknown,
}

fn fit_code(model_gb: f64, vram_gb: f64) -> ModelFit {
    if vram_gb <= 0.0 {
        ModelFit::Unknown
    } else if model_gb <= vram_gb * 0.6 {
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
        ModelFit::Unknown => 4,
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
    let hardware = catalog_hardware();
    let vram_gb = hardware.vram_bytes as f64 / 1e9;
    build_catalog(
        hardware.gpu_name,
        hardware.vram_bytes,
        vram_gb,
        &installed_names(),
    )
}

struct CatalogHardware {
    gpu_name: Option<String>,
    vram_bytes: u64,
}

#[cfg(not(target_os = "windows"))]
fn catalog_hardware() -> CatalogHardware {
    let survey = hardware::survey();
    CatalogHardware {
        gpu_name: survey.gpu_name,
        vram_bytes: survey.vram_bytes,
    }
}

#[cfg(target_os = "windows")]
fn catalog_hardware() -> CatalogHardware {
    // MeshLLM's Windows survey adds a system-RAM offload budget to discrete
    // GPU VRAM. That is useful for runtime placement, but misleading in the
    // picker: a 16 GB card with 32 GB system RAM reads as ~32 GB and receives
    // too-large recommendations. For the catalog, report and rank against
    // dedicated GPU memory only.
    select_catalog_adapter(&dxgi_adapters())
        .map(|adapter| CatalogHardware {
            gpu_name: Some(adapter.name.clone()),
            vram_bytes: adapter.dedicated_vram_bytes,
        })
        .unwrap_or(CatalogHardware {
            gpu_name: None,
            vram_bytes: 0,
        })
}

#[cfg(target_os = "windows")]
#[derive(Debug, Clone, PartialEq, Eq)]
struct DxgiAdapterInfo {
    name: String,
    dedicated_vram_bytes: u64,
    software: bool,
}

#[cfg(target_os = "windows")]
fn select_catalog_adapter(adapters: &[DxgiAdapterInfo]) -> Option<&DxgiAdapterInfo> {
    adapters
        .iter()
        .filter(|adapter| !adapter.software && adapter.dedicated_vram_bytes > 0)
        // Rank against a single adapter's memory. Summing multiple GPUs would
        // recommend models that fit in no one adapter unless the runtime can
        // explicitly shard layers across devices.
        .max_by_key(|adapter| adapter.dedicated_vram_bytes)
}

#[cfg(target_os = "windows")]
fn dxgi_adapters() -> Vec<DxgiAdapterInfo> {
    use windows::Win32::Graphics::Dxgi::{
        CreateDXGIFactory1, IDXGIFactory1, DXGI_ADAPTER_FLAG_SOFTWARE,
    };

    let Ok(factory) = (unsafe { CreateDXGIFactory1::<IDXGIFactory1>() }) else {
        return Vec::new();
    };

    let mut adapters = Vec::new();
    let mut index = 0;
    while let Ok(adapter) = unsafe { factory.EnumAdapters1(index) } {
        index += 1;
        let Ok(desc) = (unsafe { adapter.GetDesc1() }) else {
            continue;
        };
        adapters.push(DxgiAdapterInfo {
            name: utf16_description(&desc.Description),
            dedicated_vram_bytes: desc.DedicatedVideoMemory as u64,
            software: (desc.Flags & DXGI_ADAPTER_FLAG_SOFTWARE.0 as u32) != 0,
        });
    }
    adapters
}

#[cfg(target_os = "windows")]
fn utf16_description(value: &[u16]) -> String {
    let len = value.iter().position(|ch| *ch == 0).unwrap_or(value.len());
    String::from_utf16_lossy(&value[..len]).trim().to_string()
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
            let name = canonical_curated_model_id(&m.name).to_string();
            MeshCatalogEntry {
                fit: fit_code(size_gb, vram_gb),
                installed: is_installed(&m.file, &name) || is_installed(&m.file, &m.name),
                recommended: false,
                curated: false,
                name,
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
            installed: is_installed(CURATED_LARGE_FILE, CURATED_LARGE)
                || is_installed(CURATED_LARGE_FILE, CURATED_LARGE_ALIAS),
            recommended: false,
            curated: false,
            name: CURATED_LARGE.to_string(),
            size: CURATED_LARGE_SIZE.to_string(),
            size_gb,
            description: CURATED_LARGE_DESCRIPTION.to_string(),
        });
    }

    let recommended =
        (vram_bytes > 0).then(|| buzz_recommended_model(rated_capacity_gb(vram_bytes)).to_string());
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
        vram_display: if vram_bytes > 0 {
            format_rated_capacity(vram_bytes)
        } else {
            "Unknown".to_string()
        },
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
        assert_eq!(fit_code(10.0, 0.0), ModelFit::Unknown);
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
        assert_eq!(CURATED_SMALL, "unsloth/gemma-4-E4B-it-GGUF:Q4_K_M");
        assert_eq!(CURATED_LARGE, "unsloth/gemma-4-26B-A4B-it-GGUF:UD-Q4_K_M");
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

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_catalog_uses_max_dedicated_adapter_vram() {
        let adapters = vec![
            DxgiAdapterInfo {
                name: "AMD Radeon RX 7600 XT".to_string(),
                dedicated_vram_bytes: 16 * 1024 * 1024 * 1024,
                software: false,
            },
            DxgiAdapterInfo {
                name: "NVIDIA GeForce RTX 4060".to_string(),
                dedicated_vram_bytes: 8 * 1024 * 1024 * 1024,
                software: false,
            },
        ];
        let selected = select_catalog_adapter(&adapters).expect("adapter selected");
        assert_eq!(selected.name, "AMD Radeon RX 7600 XT");
        // The catalog ranks what fits on one adapter, not pooled multi-GPU VRAM.
        assert_eq!(selected.dedicated_vram_bytes, 16 * 1024 * 1024 * 1024);
        assert_eq!(
            format_rated_capacity(selected.dedicated_vram_bytes),
            "16 GB"
        );
    }

    #[test]
    fn unknown_vram_does_not_mark_entries_too_large() {
        let catalog = build_catalog(None, 0, 0.0, &[]);
        assert_eq!(catalog.vram_display, "Unknown");
        assert!(catalog.recommended.is_none());
        assert!(catalog
            .entries
            .iter()
            .all(|entry| entry.fit == ModelFit::Unknown));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_catalog_ignores_software_and_zero_vram_adapters() {
        let adapters = vec![
            DxgiAdapterInfo {
                name: "Microsoft Basic Render Driver".to_string(),
                dedicated_vram_bytes: 32 * 1024 * 1024 * 1024,
                software: true,
            },
            DxgiAdapterInfo {
                name: "DisplayLink".to_string(),
                dedicated_vram_bytes: 0,
                software: false,
            },
            DxgiAdapterInfo {
                name: "AMD Radeon RX 7600 XT".to_string(),
                dedicated_vram_bytes: 16 * 1024 * 1024 * 1024,
                software: false,
            },
        ];
        let selected = select_catalog_adapter(&adapters).expect("hardware adapter selected");
        assert_eq!(selected.name, "AMD Radeon RX 7600 XT");
    }

    #[test]
    fn curated_package_aliases_migrate_to_openai_model_ids() {
        assert_eq!(
            canonical_curated_model_id(CURATED_SMALL_ALIAS),
            CURATED_SMALL
        );
        assert_eq!(
            canonical_curated_model_id(CURATED_LARGE_ALIAS),
            CURATED_LARGE
        );
        assert_eq!(
            canonical_curated_model_id("other/model:Q4"),
            "other/model:Q4"
        );
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
