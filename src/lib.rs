pub mod message;
pub mod party;

/// General trait for value itself
pub trait Value: Eq {}

/// Trait for value selector anv verificator.
/// Value selection and verification mat depend on different conditions for different values.
pub trait ValueSelector<V: Value> {
    /// Verifies if value selected correctly
    fn verify(v: V) -> bool;

    /// Select value depending on inner conditions
    fn select() -> V;
}
