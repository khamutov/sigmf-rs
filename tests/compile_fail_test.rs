//! Guarantees that only a compiler can check.
//!
//! The seal on [`sigmf::sigmf::Sample`] is a compile-time property: it says that
//! certain code does not exist, and no test that runs can observe the absence of
//! code. `trybuild` compiles each file in `tests/ui/` and asserts it fails with the
//! error recorded in the matching `.stderr`.
//!
//! Those `.stderr` files pin diagnostics that belong to rustc, not to us, so a
//! toolchain upgrade can reword one and turn this red without anything being
//! wrong. Read the diff before believing it: if the *reason* for the rejection is
//! unchanged, regenerate with
//!
//!     TRYBUILD=overwrite cargo test --test compile_fail_test
//!
//! If the reason changed, the seal changed, and that is a real failure.

#[test]
fn sample_cannot_be_implemented_downstream() {
    trybuild::TestCases::new().compile_fail("tests/ui/*.rs");
}
