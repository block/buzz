//! Built-in Pocket voice identities and immutable asset metadata.
//!
//! Stable keys identify audio, not display labels. Future imported voices use
//! `pocket:imported:<audio-content-sha256>` and may share editable labels.

pub(super) const MARY_VOICE_KEY: &str = "pocket:mary";
pub(super) const VCTK_REVISION: &str = "323332d33f997de8394f24a193e1a76df720e01a";

pub(super) struct PocketVoiceSpec {
    pub key: &'static str,
    pub display_name: &'static str,
    pub reference_file: &'static str,
    pub upstream_file: &'static str,
    pub sha256: &'static str,
    pub bytes: Option<&'static [u8]>,
}

macro_rules! bundled_voice {
    ($key:literal, $name:literal, $file:literal, $upstream:literal, $hash:literal) => {
        PocketVoiceSpec {
            key: $key,
            display_name: $name,
            reference_file: concat!($file, ".wav"),
            upstream_file: concat!("vctk/", $upstream),
            sha256: $hash,
            bytes: Some(include_bytes!(concat!(
                "../../resources/pocket-voices/",
                $file,
                ".wav"
            ))),
        }
    };
}

/// Official English Pocket presets, in the order published by Kyutai.
pub(super) static POCKET_VOICES: &[PocketVoiceSpec] = &[
    bundled_voice!(
        "pocket:anna",
        "Anna",
        "anna",
        "p228_023_enhanced.wav",
        "0a6de25cf12bf1540beb85979f306a92be81fecc051c547c5395e7e5237a3856"
    ),
    bundled_voice!(
        "pocket:vera",
        "Vera",
        "vera",
        "p229_023_enhanced.wav",
        "309cf91a895830f15842b398f69a4962cb1f7e0bfab10e25dd27838e826c204b"
    ),
    bundled_voice!(
        "pocket:fantine",
        "Fantine",
        "fantine",
        "p244_023_enhanced.wav",
        "5f07d4e2a3f20a15572aae885156b43ef3fc12ef3812996fd135680d9956448b"
    ),
    bundled_voice!(
        "pocket:charles",
        "Charles",
        "charles",
        "p254_023_enhanced.wav",
        "6b681a429198f16e378d53bccb08d06939da7b00144a7696111d4f8f76be7756"
    ),
    bundled_voice!(
        "pocket:paul",
        "Paul",
        "paul",
        "p259_023_enhanced.wav",
        "7aba504fe0b3b16478b69eb27ce6007e3cb42b0c1915b5f1c6a6024ae37d679b"
    ),
    bundled_voice!(
        "pocket:eponine",
        "Eponine",
        "eponine",
        "p262_023_enhanced.wav",
        "a13c27fb47627b05223691a0ef2974358a18c886e6c2f9d2762ff1d02c20926b"
    ),
    bundled_voice!(
        "pocket:azelma",
        "Azelma",
        "azelma",
        "p303_023_enhanced.wav",
        "60e3d26cdf2efdec5df712152c839928f4d5522821e6554ae11fd96c57ab1026"
    ),
    bundled_voice!(
        "pocket:george",
        "George",
        "george",
        "p315_023_enhanced.wav",
        "29a41f93bf5236e5b21501091d7774c255d5f3d4e62fa4f9fdf0a92a793c84ae"
    ),
    PocketVoiceSpec {
        key: MARY_VOICE_KEY,
        display_name: "Mary",
        reference_file: "reference_sample.wav",
        upstream_file: "vctk/p333_023_enhanced.wav",
        sha256: "a35b0468382218e9f37a9a7494d1e4b74deaf18d7ced22265b4e325bb55c183f",
        bytes: None,
    },
    bundled_voice!(
        "pocket:jane",
        "Jane",
        "jane",
        "p339_023_enhanced.wav",
        "2f12e7f155eb3118f55425394f1b049e5b1b67bdc9b3932c8ba4521420aeb84a"
    ),
    bundled_voice!(
        "pocket:michael",
        "Michael",
        "michael",
        "p360_023_enhanced.wav",
        "b6743e9195e5e3fd34fe9d1633ae93f7ffab787b249e45f6467d7d6f7a6ee6ad"
    ),
    bundled_voice!(
        "pocket:eve",
        "Eve",
        "eve",
        "p361_023_enhanced.wav",
        "396e7cbd066b0f3fb6d67fa26e7904076958239d736d4390f15b5fe88feb14cd"
    ),
];

pub(super) fn source_url(voice: &PocketVoiceSpec) -> String {
    format!(
        "https://huggingface.co/kyutai/tts-voices/blob/{VCTK_REVISION}/{}",
        voice.upstream_file
    )
}
