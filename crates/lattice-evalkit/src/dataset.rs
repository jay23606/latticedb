//! Datasets: examples plus a manifest with a content hash. The loader refuses data whose hash does
//! not match, so a changed dataset can never silently move a metric.

/// One labelled example: an input and the set of labels a perfect provider returns.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Example {
    /// What the provider sees.
    pub input: String,
    /// The ground truth labels.
    pub expected: Vec<String>,
}

/// Which part of the data an example belongs to. Thresholds are set on `Dev`, reported on `Test`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Split {
    /// Used while tuning.
    Dev,
    /// Locked until a release decision.
    Test,
}

/// A hash-checked dataset.
#[derive(Debug, Clone)]
pub struct Dataset {
    /// Name.
    pub name: String,
    /// Version, bumped whenever content changes.
    pub version: u32,
    /// Which split this is.
    pub split: Split,
    /// Where the data came from (synthetic, hand-labelled, consented real).
    pub provenance: String,
    /// The examples.
    pub examples: Vec<Example>,
    /// Content hash recorded in the manifest (hex BLAKE3).
    pub hash: String,
}

/// BLAKE3 over an unambiguous framing of the examples: counts and lengths precede every field,
/// so two different datasets can never produce the same byte stream.
pub fn content_hash(examples: &[Example]) -> String {
    let mut h = blake3::Hasher::new();
    h.update(&(examples.len() as u64).to_le_bytes());
    for e in examples {
        h.update(b"E");
        h.update(&(e.input.len() as u64).to_le_bytes());
        h.update(e.input.as_bytes());
        h.update(&(e.expected.len() as u64).to_le_bytes());
        for l in &e.expected {
            h.update(&(l.len() as u64).to_le_bytes());
            h.update(l.as_bytes());
        }
    }
    h.finalize().to_hex().to_string()
}

/// Why loading failed.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum LoadError {
    /// The data does not match the hash in the manifest.
    #[error("dataset {name} v{version}: content hash {actual} does not match manifest {expected}")]
    HashMismatch {
        /// Dataset name.
        name: String,
        /// Dataset version.
        version: u32,
        /// Hash in the manifest.
        expected: String,
        /// Hash of the data.
        actual: String,
    },
    /// The manifest carries no hash at all.
    #[error("dataset {0} has no content hash in its manifest")]
    Unhashed(String),
}

impl Dataset {
    /// Builds a dataset and records its hash (for authoring tools).
    pub fn new(
        name: &str,
        version: u32,
        split: Split,
        provenance: &str,
        examples: Vec<Example>,
    ) -> Self {
        let hash = content_hash(&examples);
        Self {
            name: name.into(),
            version,
            split,
            provenance: provenance.into(),
            examples,
            hash,
        }
    }

    /// Loads examples against a manifest hash, refusing mismatches and missing hashes.
    pub fn load(
        name: &str,
        version: u32,
        split: Split,
        provenance: &str,
        examples: Vec<Example>,
        manifest_hash: Option<&str>,
    ) -> Result<Self, LoadError> {
        let expected = manifest_hash.ok_or_else(|| LoadError::Unhashed(name.to_string()))?;
        let actual = content_hash(&examples);
        if actual != expected {
            return Err(LoadError::HashMismatch {
                name: name.into(),
                version,
                expected: expected.to_string(),
                actual,
            });
        }
        Ok(Self {
            name: name.into(),
            version,
            split,
            provenance: provenance.into(),
            examples,
            hash: actual,
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ex(i: &str, l: &[&str]) -> Example {
        Example {
            input: i.into(),
            expected: l.iter().map(|s| (*s).to_string()).collect(),
        }
    }

    #[test]
    fn modified_or_unhashed_data_is_refused() {
        let good = vec![ex("a", &["x"]), ex("b", &[])];
        let d = Dataset::new("d", 1, Split::Dev, "synthetic", good.clone());
        assert!(Dataset::load("d", 1, Split::Dev, "synthetic", good, Some(&d.hash)).is_ok());
        let changed = vec![ex("a", &["y"]), ex("b", &[])];
        assert!(matches!(
            Dataset::load("d", 1, Split::Dev, "synthetic", changed, Some(&d.hash)),
            Err(LoadError::HashMismatch { .. })
        ));
        assert_eq!(
            Dataset::load("d", 1, Split::Dev, "synthetic", vec![], None).unwrap_err(),
            LoadError::Unhashed("d".into())
        );
    }

    #[test]
    fn hash_distinguishes_boundaries() {
        assert_ne!(
            content_hash(&[ex("ab", &["c"])]),
            content_hash(&[ex("a", &["bc"])])
        );
        assert_ne!(
            content_hash(&[ex("a", &["b", "c"])]),
            content_hash(&[ex("a", &["bc"])])
        );
    }
}
