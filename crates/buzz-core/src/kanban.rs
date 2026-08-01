//! Kanban rank ordering — base-36 order-preserving fractional ranks.
//!
//! Card ordering within a Kanban column is a `rank` string. Using raw decimal
//! "midpoints" misorders (`"0.10" < "0.9"` as text but `0.10 > 0.9` as
//! numbers). Instead we use a base-36 fractional rank with the invariant that
//! a valid rank never ends in the smallest digit `'0'` (canonical, injective).
//! With that invariant, plain lexicographic string comparison EQUALS numeric
//! value order, so the comparator is an ordinary string `<`.
//!
//! Operations produce unbounded interstitial insertion with no mass
//! renumbering: `first` (a column's first card), `rank_after`/`rank_before`
//! (insert at head/tail), and `rank_between` (reorder between two cards).
//! Ported from the validated reference at
//! `PLANS/KANBAN_RANK_CODEC/rank.js` (5/5 tests there).

const DIGITS: &str = "0123456789abcdefghijklmnopqrstuvwxyz";

fn digit_index(c: u8) -> usize {
    DIGITS
        .find(c as char)
        .unwrap_or_else(|| panic!("kanban rank digit out of alphabet: {c}"))
}

fn digit_char(i: usize) -> char {
    DIGITS.as_bytes()[i] as char
}

/// A valid canonical rank: non-empty, all in the alphabet, not ending in '0'.
pub fn is_valid(rank: &str) -> bool {
    !rank.is_empty()
        && !rank.ends_with(DIGITS.as_bytes()[0] as char)
        && rank.bytes().all(|b| DIGITS.find(b as char).is_some())
}

/// Smallest (non-empty, canonical) valid rank strictly below `x`.
/// `x` must be a valid rank (never called with the open boundary on this path).
fn below(x: &str) -> String {
    let first = x.as_bytes()[0];
    if first == DIGITS.as_bytes()[0] {
        // Prepend a '0' and go deeper: "0" + below(rest)
        let mut out = String::from("0");
        out.push_str(&below(&x[1..]));
        out
    } else if first == b'1' {
        // "01" < any "1...."
        String::from("01")
    } else {
        digit_char(digit_index(first) - 1).to_string()
    }
}

/// A valid canonical rank strictly above `x` (compact successor).
fn above(x: &str) -> String {
    let bytes = x.as_bytes();
    let max = DIGITS.as_bytes()[DIGITS.len() - 1];
    for i in (0..bytes.len()).rev() {
        if bytes[i] < max {
            let mut out = String::from(&x[..i]);
            out.push(digit_char(digit_index(bytes[i]) + 1));
            return out;
        }
    }
    // all max chars -> extend
    let mut out = String::from(x);
    out.push('1');
    out
}

/// Valid rank strictly between `lo` and `hi` (open bounds passed as `""`).
pub fn between(lo: &str, hi: &str) -> String {
    if lo.is_empty() && hi.is_empty() {
        return digit_char(DIGITS.len() >> 1).to_string(); // 'i' (mid of alphabet)
    }
    if lo.is_empty() {
        return below(hi);
    }
    if hi.is_empty() {
        return above(lo);
    }

    let lb = lo.as_bytes();
    let hb = hi.as_bytes();
    let n = lb.len().min(hb.len());
    let mut p = 0;
    while p < n && lb[p] == hb[p] {
        p += 1;
    }

    let tail_lo = &lo[p..];
    if tail_lo.is_empty() {
        // lo is a prefix of hi; extend lo below hi's remainder
        let mut out = String::from(&lo[..p]);
        out.push_str(&below(&hi[p..]));
        return out;
    }
    // diverge: lo[p] < hi[p] (since lo < hi); extending lo stays < hi
    let mut out = String::from(lo);
    out.push('1');
    out
}

/// The rank of a column's first card.
pub fn first_rank() -> String {
    between("", "")
}

/// A rank strictly before `rank` (insert at head of a column).
pub fn rank_before(rank: &str) -> String {
    between("", rank)
}

/// A rank strictly after `rank` (insert at tail of a column).
pub fn rank_after(rank: &str) -> String {
    between(rank, "")
}

/// A rank strictly between `a` and `b` (reorder between two adjacent cards).
pub fn rank_between(a: &str, b: &str) -> String {
    between(a, b)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn assert_canonical(r: &str) {
        assert!(is_valid(r), "not canonical/valid: {r:?}");
    }

    #[test]
    fn validity() {
        assert!(is_valid("i"));
        assert!(is_valid("ab1"));
        assert!(is_valid("001"));
        assert!(!is_valid(""));
        assert!(!is_valid("a0")); // trailing smallest digit
        assert!(!is_valid("a!")); // out of alphabet
    }

    #[test]
    fn first_and_bounds() {
        let f = first_rank();
        assert_canonical(&f);
        let mut prev = f.clone();
        for _ in 0..200 {
            let y = rank_before(&prev);
            assert_canonical(&y);
            assert!(y.as_str() < prev.as_str());
            prev = y;
        }
        prev = first_rank();
        for _ in 0..200 {
            let y = rank_after(&prev);
            assert_canonical(&y);
            assert!(y.as_str() > prev.as_str());
            prev = y;
        }
    }

    #[test]
    fn strict_betweenness() {
        let keys = ["i", "h", "g", "01", "001", "j", "ab", "ac", "az", "ab1", "zz1", "z", "0a9", "i1"];
        for a in keys {
            for b in keys {
                if a == b {
                    continue;
                }
                let (lo, hi) = if a < b { (a, b) } else { (b, a) };
                let c = rank_between(lo, hi);
                assert_canonical(&c);
                assert!(c.as_str() > lo, "c={c} not > lo={lo}");
                assert!(c.as_str() < hi, "c={c} not < hi={hi}");
            }
        }
    }

    #[test]
    fn fuzz_interleaved_inserts() {
        // deterministic LCG fuzz
        let mut seed: u64 = 123456789;
        let mut rnd = move || {
            seed = seed.wrapping_mul(6364136223846793005).wrapping_add(1442695040888963407);
            (seed >> 33) as usize
        };
        let mut keys: Vec<String> = vec![first_rank()];
        for _ in 0..5000 {
            let idx = rnd() % (keys.len() + 1);
            let lo = if idx == 0 { String::new() } else { keys[idx - 1].clone() };
            let hi = if idx == keys.len() { String::new() } else { keys[idx].clone() };
            let nk = between(&lo, &hi);
            assert_canonical(&nk);
            if !lo.is_empty() {
                assert!(nk.as_str() > lo.as_str());
            }
            if !hi.is_empty() {
                assert!(nk.as_str() < hi.as_str());
            }
            keys.insert(idx, nk);
        }
        for i in 0..keys.len() {
            assert_canonical(&keys[i]);
            if i > 0 {
                assert!(keys[i].as_str() > keys[i - 1].as_str(), "unsorted at {i}");
            }
        }
    }
}
