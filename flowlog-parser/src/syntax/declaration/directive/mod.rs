//! `.input`, `.output`, `.printsize`, and `.limitsize` directives.
//!
//! One leaf per directive, each also holding the resolved form its
//! parameters take, which the enclosing [`Relation`](super::Relation)
//! adopts: [`InputSource`] for `.input`, [`OutputSink`] for `.output`.

mod input;
mod limitsize;
mod output;
mod param;
mod printsize;

pub(crate) use input::InputDirective;
pub use input::InputSource;
pub(crate) use limitsize::LimitSizeDirective;
pub use output::OrderKey;
pub(crate) use output::OutputDirective;
pub use output::OutputSink;
pub(in crate::syntax::declaration) use param::parse_io_directive;
pub(crate) use printsize::PrintSizeDirective;
