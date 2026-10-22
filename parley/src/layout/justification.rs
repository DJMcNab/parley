// Copyright 2026 the Parley Authors
// SPDX-License-Identifier: Apache-2.0 OR MIT

//! Immutable ownership of justification opportunities, built once per set of lines.

use alloc::vec::Vec;
use parley_engine::{Atom, ShapedText};

use super::data::{LayoutItemKind, LineData, LineItemData};
use super::spacing::{Gaps, is_word_separator};

/// How much of an opportunity an atom owns on one visual side.
#[derive(Clone, Copy, Debug, PartialEq)]
#[repr(u8)]
enum Share {
    None,
    #[expect(
        dead_code,
        reason = "Reserved for shared boundary ownership in CJ justification"
    )]
    Half,
    Whole,
}

impl Share {
    fn amount(self) -> f32 {
        match self {
            Self::None => 0.,
            Self::Half => 0.5,
            Self::Whole => 1.,
        }
    }
}

/// One owning atom, keyed by its first shaped cluster.
#[derive(Clone, Copy, Debug, PartialEq)]
struct Record {
    cluster: u32,
    before: Share,
    after: Share,
}

impl Record {
    fn gaps(self, amount: f32) -> Gaps {
        Gaps {
            before: self.before.amount() * amount,
            after: self.after.amount() * amount,
        }
    }
}

/// Layout-owned storage. Clearing retains capacity across line breaking.
#[derive(Clone, Debug, Default, PartialEq)]
pub(crate) struct JustificationLedger {
    records: Vec<Record>,
    built: bool,
}

impl JustificationLedger {
    pub(crate) fn clear(&mut self) {
        self.records.clear();
        self.built = false;
    }

    /// Walk each line's atoms in visual order. Only word separators own opportunities for now:
    /// each non-hanging separator owns one whole opportunity visually after itself, independently
    /// of its neighbours. No eligibility decisions are made by consumers of the resulting ledger.
    pub(crate) fn build(
        &mut self,
        shaped_text: &ShapedText,
        line_items: &[LineItemData],
        lines: &mut [LineData],
    ) {
        if self.built {
            return;
        }
        for line in lines {
            let start = self.records.len();
            for item in &line_items[line.item_range.clone()] {
                if item.kind != LayoutItemKind::TextRun {
                    continue;
                }
                let slice = shaped_text
                    .run_slice(u32::try_from(item.index).unwrap())
                    .narrow(item.shaped_cluster_range.clone());
                let mut atoms = if item.is_rtl() {
                    slice.atoms_end()
                } else {
                    slice.atoms_start()
                };
                while let Some(atom) = if item.is_rtl() {
                    atoms.prev()
                } else {
                    atoms.next()
                } {
                    let cluster = atom.shaped_clusters_range().start;
                    if cluster < line.justification_end_cluster
                        && is_word_separator(atom.characters()[0].whitespace)
                    {
                        self.records.push(Record {
                            cluster,
                            before: Share::None,
                            after: Share::Whole,
                        });
                    }
                }
            }
            self.records[start..].sort_unstable_by_key(|record| record.cluster);
            line.justification =
                u32::try_from(start).unwrap()..u32::try_from(self.records.len()).unwrap();
        }
        self.built = true;
    }

    pub(crate) fn line(&self, line: &LineData) -> Justification<'_> {
        Justification {
            amount_per_opportunity: line.justification_amount,
            records: &self.records
                [line.justification.start as usize..line.justification.end as usize],
        }
    }
}

/// A line's ownership decisions and its current expansion per opportunity.
#[derive(Clone, Copy, Debug)]
pub(crate) struct Justification<'a> {
    amount_per_opportunity: f32,
    records: &'a [Record],
}

impl<'a> Justification<'a> {
    pub(crate) const NONE: Self = Self {
        amount_per_opportunity: 0.,
        records: &[],
    };

    pub(crate) fn opportunities(self) -> f32 {
        self.records
            .iter()
            .map(|record| record.before.amount() + record.after.amount())
            .sum()
    }

    pub(crate) fn is_zero(self) -> bool {
        self.amount_per_opportunity == 0.
    }

    /// Random-access lookup, for cluster advances and interaction geometry.
    pub(crate) fn gaps(self, atom: &Atom<'_>) -> Gaps {
        if self.is_zero() {
            return Gaps::ZERO;
        }
        self.records
            .binary_search_by_key(&atom.shaped_clusters_range().start, |record| record.cluster)
            .map_or(Gaps::ZERO, |index| {
                self.records[index].gaps(self.amount_per_opportunity)
            })
    }

    pub(crate) fn cursor(self, is_rtl: bool) -> JustificationCursor<'a> {
        JustificationCursor {
            justification: self,
            is_rtl,
            index: None,
        }
    }
}

/// Per-run sequential lookup: seek once, then advance through the sorted records. Repeated queries
/// for the same atom are allowed (e.g. when peeking across a glyph-run boundary).
#[derive(Clone, Copy, Debug)]
pub(crate) struct JustificationCursor<'a> {
    justification: Justification<'a>,
    is_rtl: bool,
    index: Option<usize>,
}

impl JustificationCursor<'_> {
    pub(crate) fn is_zero(&self) -> bool {
        self.justification.is_zero()
    }
    pub(crate) fn gaps(&mut self, atom: &Atom<'_>) -> Gaps {
        if self.is_zero() {
            return Gaps::ZERO;
        }
        let cluster = atom.shaped_clusters_range().start;
        let records = self.justification.records;
        let index = self.index.get_or_insert_with(|| {
            records.partition_point(|record| {
                if self.is_rtl {
                    record.cluster <= cluster
                } else {
                    record.cluster < cluster
                }
            })
        });
        let record = if self.is_rtl {
            while *index > 0 && records[*index - 1].cluster > cluster {
                *index -= 1;
            }
            index.checked_sub(1).map(|index| &records[index])
        } else {
            while *index < records.len() && records[*index].cluster < cluster {
                *index += 1;
            }
            records.get(*index)
        };
        record
            .filter(|record| record.cluster == cluster)
            .map_or(Gaps::ZERO, |record| {
                record.gaps(self.justification.amount_per_opportunity)
            })
    }
}

#[cfg(test)]
mod tests;
