//! German text normalization for Kokoro TTS.
//!
//! Ported from the v1.1/v1.2 rule set shipped with
//! `Godelaune/Kokoro-82M-ONNX-German-Martin` (`german_text_rules.py`) and
//! extended for dates, times, currency, percentages, and number words.

use regex::Regex;
use std::sync::OnceLock;

pub fn normalize_german_text(input: &str) -> String {
    let mut text = input.replace('\u{00a0}', " ");
    text = expand_dates(&text);
    text = expand_times(&text);
    text = expand_currency(&text);
    text = expand_percentages(&text);
    text = expand_thousands(&text);
    text = expand_decimal_units(&text);
    text = expand_abbreviations(&text);
    text = expand_remaining_numbers(&text);
    collapse_ws(&text)
}

fn expand_abbreviations(text: &str) -> String {
    let mut out = text.to_string();
    for (pattern, replacement) in ABBREVIATIONS {
        let re = compiled(pattern);
        out = re.replace_all(&out, *replacement).into_owned();
    }
    out
}

fn expand_dates(text: &str) -> String {
    let re = compiled(r"\b(\d{1,2})\.(\d{1,2})\.(\d{2,4})\b");
    re.replace_all(text, |caps: &regex::Captures| {
        let day = caps[1].parse::<u32>().unwrap_or(0);
        let month = caps[2].parse::<u32>().unwrap_or(0);
        let year = normalize_year(&caps[3]);
        format!(
            "{} {} {}",
            ordinal_masculine(day),
            month_name(month),
            year_words(year)
        )
    })
    .into_owned()
}

fn expand_times(text: &str) -> String {
    let re = compiled(r"\b(\d{1,2}):(\d{2})\s*(Uhr)?\b");
    re.replace_all(text, |caps: &regex::Captures| {
        let hour = caps[1].parse::<u32>().unwrap_or(0);
        let minute = caps[2].parse::<u32>().unwrap_or(0);
        if minute == 0 {
            format!("{} Uhr", cardinal(hour))
        } else {
            format!("{} Uhr {}", cardinal(hour), cardinal(minute))
        }
    })
    .into_owned()
}

fn expand_currency(text: &str) -> String {
    let re = compiled(r"\b(\d{1,3}(?:\.\d{3})*|\d+)(?:,(\d{1,2}))?\s*(?:€|EUR|Euro)\b?");
    re.replace_all(text, |caps: &regex::Captures| {
        let euros = parse_grouped_int(&caps[1]);
        let cents = caps
            .get(2)
            .map(|m| m.as_str())
            .filter(|s| !s.is_empty())
            .map(|s| {
                if s.len() == 1 {
                    s.parse::<u32>().unwrap_or(0) * 10
                } else {
                    s.parse::<u32>().unwrap_or(0)
                }
            })
            .unwrap_or(0);
        if cents == 0 {
            format!("{} Euro", cardinal(euros))
        } else {
            format!("{} Euro {}", cardinal(euros), cardinal(cents))
        }
    })
    .into_owned()
}

fn expand_percentages(text: &str) -> String {
    let re = compiled(r"\b(\d+)(?:,(\d+))?\s*%");
    re.replace_all(text, |caps: &regex::Captures| {
        let whole = caps[1].parse::<u32>().unwrap_or(0);
        match caps.get(2) {
            Some(frac) if !frac.as_str().is_empty() => {
                format!(
                    "{} komma {} Prozent",
                    cardinal(whole),
                    digit_run(frac.as_str())
                )
            }
            _ => format!("{} Prozent", cardinal(whole)),
        }
    })
    .into_owned()
}

fn expand_thousands(text: &str) -> String {
    let re = compiled(r"\b(\d{1,3}(?:\.\d{3})+)\b");
    re.replace_all(text, |caps: &regex::Captures| {
        cardinal(parse_grouped_int(&caps[1]))
    })
    .into_owned()
}

fn expand_decimal_units(text: &str) -> String {
    let mut out = text.to_string();
    for (unit, singular, plural, _) in NUMBERED_UNITS {
        let pattern = format!(r"\b(\d+)(?:,(\d+))?\s*{unit}\b");
        let re = compiled(&pattern);
        out = re
            .replace_all(&out, |caps: &regex::Captures| {
                let whole = caps[1].parse::<u32>().unwrap_or(0);
                let frac = caps.get(2).map(|m| m.as_str()).unwrap_or("");
                let number = if frac.is_empty() {
                    cardinal(whole)
                } else {
                    format!("{} komma {}", cardinal(whole), digit_run(frac))
                };
                let unit_word = if whole == 1 && frac.is_empty() {
                    *singular
                } else {
                    *plural
                };
                format!("{number} {unit_word}")
            })
            .into_owned();
    }
    out
}

fn expand_remaining_numbers(text: &str) -> String {
    let re = compiled(r"\b\d+\b");
    re.replace_all(text, |caps: &regex::Captures| {
        caps[0]
            .parse::<u32>()
            .map(cardinal)
            .unwrap_or_else(|_| caps[0].to_string())
    })
    .into_owned()
}

fn normalize_year(raw: &str) -> u32 {
    let year = raw.parse::<u32>().unwrap_or(0);
    if raw.len() == 2 {
        if year >= 70 {
            1900 + year
        } else {
            2000 + year
        }
    } else {
        year
    }
}

fn year_words(year: u32) -> String {
    if (1100..2000).contains(&year) {
        let century = year / 100;
        let rest = year % 100;
        if rest == 0 {
            format!("{}hundert", cardinal(century))
        } else {
            format!("{}hundert{}", cardinal(century), cardinal(rest))
        }
    } else if (2000..2100).contains(&year) {
        if year == 2000 {
            "zweitausend".to_string()
        } else {
            format!("zweitausend{}", cardinal(year - 2000))
        }
    } else {
        cardinal(year)
    }
}

fn month_name(month: u32) -> &'static str {
    match month {
        1 => "Januar",
        2 => "Februar",
        3 => "März",
        4 => "April",
        5 => "Mai",
        6 => "Juni",
        7 => "Juli",
        8 => "August",
        9 => "September",
        10 => "Oktober",
        11 => "November",
        12 => "Dezember",
        _ => "",
    }
}

fn ordinal_masculine(n: u32) -> String {
    match n {
        1 => "erster".into(),
        3 => "dritter".into(),
        7 => "siebter".into(),
        8 => "achter".into(),
        _ => format!("{}ter", cardinal(n)),
    }
}

fn parse_grouped_int(raw: &str) -> u32 {
    raw.replace('.', "").parse().unwrap_or(0)
}

fn digit_run(raw: &str) -> String {
    raw.chars()
        .filter_map(|c| c.to_digit(10).map(cardinal))
        .collect::<Vec<_>>()
        .join(" ")
}

fn collapse_ws(text: &str) -> String {
    text.split_whitespace().collect::<Vec<_>>().join(" ")
}

fn compiled(pattern: &str) -> Regex {
    static CACHE: OnceLock<std::sync::Mutex<Vec<(String, Regex)>>> = OnceLock::new();
    let cache = CACHE.get_or_init(|| std::sync::Mutex::new(Vec::new()));
    let mut guard = cache.lock().unwrap_or_else(|e| e.into_inner());
    if let Some((_, re)) = guard.iter().find(|(p, _)| p == pattern) {
        return re.clone();
    }
    let re = Regex::new(pattern).expect("german normalize regex");
    guard.push((pattern.to_string(), re.clone()));
    re
}

const ABBREVIATIONS: &[(&str, &str)] = &[
    (r"(?i)\bLfd\.\s*Nr\.", "laufende Nummer"),
    (r"(?i)\bz\s*\.?\s*B\.", "zum Beispiel"),
    (r"(?i)\bzB\b", "zum Beispiel"),
    (r"(?i)\betc\.", "ezetera"),
    (r"(?i)\busw\.", "und so weiter"),
    (r"(?i)\bca\.", "zirka"),
    (r"(?i)\bd\.\s*h\.", "das heißt"),
    (r"(?i)\bu\.\s*a\.", "unter anderem"),
    (r"(?i)\bggf\.", "gegebenenfalls"),
    (r"(?i)\bzzgl\.", "zuzüglich"),
    (r"(?i)\bDr\.", "Doktor"),
    (r"(?i)\bProf\.", "Professor"),
    (r"(?i)\bAbk\.", "Abkürzung"),
    (r"(?i)\bAbb\.", "Abbildung"),
    (r"(?i)\bgeb\.", "geboren"),
    (r"(?i)\bbspw\.", "beispielsweise"),
    (r"(?i)\bNr\.", "Nummer"),
    (r"(?i)\bggü\.", "gegenüber"),
    (r"(?i)\bKap\.", "Kapitel"),
    (r"(?i)\bAbs\.", "Absatz"),
    (r"(?i)\bTsd\.", "Tausend"),
    (r"(?i)\bMio\.", "Millionen"),
    (r"(?i)\bMrd\.", "Milliarden"),
    (r"\bGmbH\b", "G-M-B-H"),
    (r"\bAG\b", "A-G"),
];

const NUMBERED_UNITS: &[(&str, &str, &str, Option<&str>)] = &[
    (r"kWh", "Kilowattstunde", "Kilowattstunden", Some("f")),
    (r"Wh", "Wattstunde", "Wattstunden", Some("f")),
    (r"GHz", "Gigahertz", "Gigahertz", None),
    (r"MHz", "Megahertz", "Megahertz", None),
    (r"kHz", "Kilohertz", "Kilohertz", None),
    (r"Hz", "Hertz", "Hertz", None),
    (r"Std\.", "Stunde", "Stunden", Some("f")),
    (r"Min\.", "Minute", "Minuten", Some("f")),
    (r"Sek\.", "Sekunde", "Sekunden", Some("f")),
    (r"Stck\.", "Stück", "Stück", Some("n")),
    (r"mAh", "Milliamperestunde", "Milliamperestunden", Some("f")),
    (r"mA", "Milliampere", "Milliampere", None),
    (r"kg", "Kilogramm", "Kilogramm", None),
    (r"km", "Kilometer", "Kilometer", None),
    (r"cm", "Zentimeter", "Zentimeter", None),
    (r"mm", "Millimeter", "Millimeter", None),
    (r"ltr\.", "Liter", "Liter", None),
    (r"EUR", "Euro", "Euro", None),
    (r"g", "Gramm", "Gramm", None),
    (r"m", "Meter", "Meter", None),
    (r"W", "Watt", "Watt", None),
    (r"V", "Volt", "Volt", None),
];

fn cardinal(n: u32) -> String {
    if n == 0 {
        return "null".into();
    }
    if n < 20 {
        return ONES[n as usize].into();
    }
    if n < 100 {
        let ones = n % 10;
        let tens = n / 10;
        if ones == 0 {
            return TENS[tens as usize].into();
        }
        return format!("{}und{}", ONES_JOIN[ones as usize], TENS[tens as usize]);
    }
    if n < 1000 {
        let hundreds = n / 100;
        let rest = n % 100;
        let head = if hundreds == 1 {
            "einhundert".to_string()
        } else {
            format!("{}hundert", cardinal(hundreds))
        };
        if rest == 0 {
            head
        } else {
            format!("{head}{}", cardinal(rest))
        }
    } else if n < 1_000_000 {
        let thousands = n / 1000;
        let rest = n % 1000;
        let head = if thousands == 1 {
            "eintausend".to_string()
        } else {
            format!("{}tausend", cardinal(thousands))
        };
        if rest == 0 {
            head
        } else {
            format!("{head}{}", cardinal(rest))
        }
    } else {
        n.to_string()
    }
}

const ONES: [&str; 20] = [
    "null",
    "eins",
    "zwei",
    "drei",
    "vier",
    "fünf",
    "sechs",
    "sieben",
    "acht",
    "neun",
    "zehn",
    "elf",
    "zwölf",
    "dreizehn",
    "vierzehn",
    "fünfzehn",
    "sechzehn",
    "siebzehn",
    "achtzehn",
    "neunzehn",
];

const ONES_JOIN: [&str; 10] = [
    "", "ein", "zwei", "drei", "vier", "fünf", "sechs", "sieb", "acht", "neun",
];

const TENS: [&str; 10] = [
    "", "", "zwanzig", "dreißig", "vierzig", "fünfzig", "sechzig", "siebzig", "achtzig", "neunzig",
];

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn normalizes_required_german_examples() {
        let cases = [
            ("14.05.2026", "vierzehnter Mai zweitausendsechsundzwanzig"),
            ("18:20 Uhr", "achtzehn Uhr zwanzig"),
            ("49,99 €", "neunundvierzig Euro neunundneunzig"),
            ("1,5 kg", "eins komma fünf Kilogramm"),
            ("25 %", "fünfundzwanzig Prozent"),
            ("Dr. Müller", "Doktor Müller"),
            ("z. B.", "zum Beispiel"),
            ("Straße", "Straße"),
            ("München", "München"),
            ("1.234", "eintausendzweihundertvierunddreißig"),
        ];
        for (input, expected) in cases {
            assert_eq!(normalize_german_text(input), expected, "input={input}");
        }
    }

    #[test]
    fn preserves_eszett_and_umlauts() {
        assert_eq!(normalize_german_text("Straße"), "Straße");
        assert_eq!(normalize_german_text("München"), "München");
    }
}
