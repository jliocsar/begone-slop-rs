//! begone-slop for Rust: lint rules that reject low-evidence code.
//!
//! The binary is `cargo bs`. This library half exists so the repo's own tests can run the
//! syntax rules over its sources, which is how the tool checks itself.

pub mod ast_grep;
pub mod check;
pub mod files;
pub mod init;
pub mod rules;
pub mod source;
