// QUIC session coordinator - delegates to domain layer
//
// This module now acts as a thin adapter to the domain session crate.
// The actual session state management is in mrd-session.

pub use mrd_session::{QuicSessionCoordinator, QuicSessionSnapshot};
