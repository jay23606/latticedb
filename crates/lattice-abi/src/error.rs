//! The error taxonomy every crate error maps onto.

/// How a caller must react to an error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum ErrorClass {
    /// Safe to retry the same request later.
    Retryable,
    /// The request is invalid or not allowed; retrying will not help.
    Rejected,
    /// The component can no longer be trusted (for example a failed fsync or corruption). Stop.
    Fatal,
    /// A database invariant was found violated. This is a bug.
    InvariantViolation,
}

/// Implemented by every crate error enum so callers can react without matching variants.
pub trait Classify {
    /// The class of this error.
    fn class(&self) -> ErrorClass;
}
