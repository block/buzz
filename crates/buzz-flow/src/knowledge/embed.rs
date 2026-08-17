//! Deterministic text → vector embedding (MVP semantic search without an external model).

/// OpenAI-compatible embedding width used by migration 0032.
pub const EMBEDDING_DIM: usize = 1536;

/// Hash token activations into a fixed-size vector, then L2-normalize.
///
/// Not a production embedding model — sufficient for dev/MVP cosine ranking over
/// ingested Flow Studio documents until a real model pipeline is wired.
pub fn text_to_embedding(text: &str) -> Vec<f32> {
    let mut values = vec![0.0f32; EMBEDDING_DIM];
    for token in text.split_whitespace() {
        let normalized = token.to_ascii_lowercase();
        if normalized.is_empty() {
            continue;
        }
        let hash = fnv1a64(normalized.as_bytes());
        let idx = (hash as usize) % EMBEDDING_DIM;
        values[idx] += 1.0;
        let idx2 = ((hash >> 32) as usize) % EMBEDDING_DIM;
        values[idx2] += 0.5;
    }
    l2_normalize(&mut values);
    values
}

/// Format a vector for Postgres `pgvector` query parameters.
pub fn embedding_to_pgvector(values: &[f32]) -> String {
    let inner: Vec<String> = values.iter().map(|v| v.to_string()).collect();
    format!("[{}]", inner.join(","))
}

fn fnv1a64(bytes: &[u8]) -> u64 {
    const OFFSET: u64 = 0xcbf29ce484222325;
    const PRIME: u64 = 0x00000100000001B3;
    let mut hash = OFFSET;
    for byte in bytes {
        hash ^= u64::from(*byte);
        hash = hash.wrapping_mul(PRIME);
    }
    hash
}

fn l2_normalize(values: &mut [f32]) {
    let sum_sq: f32 = values.iter().map(|v| v * v).sum();
    if sum_sq <= f32::EPSILON {
        return;
    }
    let norm = sum_sq.sqrt();
    for value in values {
        *value /= norm;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn embedding_is_normalized() {
        let embedding = text_to_embedding("Buzz Hive knowledge base");
        assert_eq!(embedding.len(), EMBEDDING_DIM);
        let sum_sq: f32 = embedding.iter().map(|v| v * v).sum();
        assert!((sum_sq - 1.0).abs() < 0.001);
    }

    #[test]
    fn similar_text_has_higher_dot_than_unrelated() {
        let a = text_to_embedding("rust workflow automation");
        let b = text_to_embedding("rust workflow engine");
        let c = text_to_embedding("chocolate cake recipe");
        let ab = dot(&a, &b);
        let ac = dot(&a, &c);
        assert!(ab > ac);
    }

    fn dot(a: &[f32], b: &[f32]) -> f32 {
        a.iter().zip(b).map(|(x, y)| x * y).sum()
    }
}
