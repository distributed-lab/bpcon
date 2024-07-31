pub mod message;
pub mod party;
mod error;

/// General trait for value itself.
pub trait Value: Eq {}

/// Trait for value selector and verificator.
/// Value selection and verification may depend on different conditions for different values.
pub trait ValueSelector<V: Value> {
    /// Verifies if a value is selected correctly.
    fn verify(v: V) -> bool;

    /// Select value depending on inner conditions.
    fn select() -> V;

    // TODO: add other fields to update selector state.
}
