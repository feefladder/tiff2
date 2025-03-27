#!/usr/bin/env rust-script
//! just some testing
use std::collections::BTreeMap;
use std::ops::{Range, Deref};
use std::cmp::Ordering;

#[derive(Debug, PartialEq, Eq)]
struct FileRange(Range<u64>);

impl Deref for FileRange {
    type Target = Range<u64>;
    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl PartialOrd for FileRange {
    fn partial_cmp(&self, other: &Self) -> Option<Ordering> {
        Some(self.cmp(other))
    }
}

impl Ord for FileRange {
    fn cmp(&self, other: &Self) -> Ordering {
        if self.end == other.end {
            Ordering::Equal
        } else if self.end < other.end {
            Ordering::Less
        } else {
            Ordering::Greater
        }
    }
}

fn main() {
    let mut tracker = BTreeMap::new();
    tracker.insert(FileRange(0..5u64), 42u64);
    
}