//! `Sample` must not be implementable downstream.
//!
//! This type is eight bytes wide. If the impl below were accepted, it could go on
//! to claim any `core:datatype` it liked — `rf32_le`, four bytes — and every
//! recording written through it would misdescribe its own Dataset while passing
//! through a signature that looks like it checked.

use sigmf::sigmf::Sample;

#[derive(Clone, Copy)]
struct EightByteThing {
    _bits: f64,
}

impl Sample for EightByteThing {}

fn main() {}
