//! Dual-format MOBI orchestration and serialization.

mod layout;
mod resource;
mod serializer;

pub(crate) use serializer::serialize_to_writer;
