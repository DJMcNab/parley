// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: MIT

// Copyright 2026 Christian Hansen
// SPDX-License-Identifier: MIT
// <https://github.com/chansen/c-emoji>

// After you edit the crate's doc comment, run this command, then check README.md for any missing links
// cargo rdme --workspace-project=parley_emoji

//! Emoji presentation resolution for text layout.
//!
//! Some Unicode characters and sequences can be displayed either as ordinary
//! text glyphs or as emoji. This crate determines the preferred presentation
//! of an already-delimited text cluster from its Unicode emoji properties,
//! variation selectors, and sequence structure.
//!
//! In Parley, this happens before font selection. Clusters with emoji
//! presentation can use an emoji font fallback, while clusters with text
//! presentation remain in the ordinary text font stack.
//!
//! The implementation recognizes the structural emoji sequences defined by
//! [Unicode Technical Standard #51][UTS51] using a DFA based on [c-emoji][].
//! It does not perform grapheme segmentation, font selection, or shaping.
//!
//! # Features
//!
//! The following crate [feature flags](https://doc.rust-lang.org/cargo/reference/features.html#dependency-features) are available:
//!
//! - `std` (enabled by default): This is currently unused and is provided for forward compatibility.
//!
//! Note that Parley Emoji does require that an allocator is available (i.e. it uses [`alloc`][]).
//!
//! [c-emoji]: <https://github.com/chansen/c-emoji>
//! [UTS51]: <https://www.unicode.org/reports/tr51/>

// LINEBENDER LINT SET - lib.rs - v4
// See https://linebender.org/wiki/canonical-lints/
// These lints shouldn't apply to examples or tests.
#![cfg_attr(not(test), warn(unused_crate_dependencies))]
// These lints shouldn't apply to examples.
#![warn(clippy::print_stdout, clippy::print_stderr)]
// Targeting e.g. 32-bit means structs containing usize can give false positives for 64-bit.
#![cfg_attr(target_pointer_width = "64", warn(clippy::trivially_copy_pass_by_ref))]
// END LINEBENDER LINT SET
#![cfg_attr(docsrs, feature(doc_cfg))]
#![no_std]

// Avoid adding alloc in the future being a breaking change.
extern crate alloc as _;
// Ensure that we don't compile if you're using the std feature on a platform without `std`
#[cfg(feature = "std")]
extern crate std as _;

mod dfa;
mod types;

pub use dfa::EmojiDFA;
pub use types::{EmojiPresentationStyle, EmojiSegmentationCategory};
