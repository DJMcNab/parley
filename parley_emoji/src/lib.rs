// Copyright 2026 the Parley Authors and Christian Hansen
// SPDX-License-Identifier: MIT

// Adapted from <https://github.com/chansen/c-emoji>

//! Port of [c-emoji] (MIT) and follow the [UTS51](Unicode Technical Standard #51).
//!
//! [c-emoji]: <https://github.com/chansen/c-emoji>
//! [UTS51]: <https://www.unicode.org/reports/tr51/>

#![no_std]

mod dfa;
mod types;

pub use dfa::EmojiDFA;
pub use types::{EmojiPresentationStyle, EmojiSegmentationCategory};
