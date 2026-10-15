//! Regression tests for soundness issues.
//!
//! Most of these only detect the problem under Miri (`cargo +nightly miri test --test soundness`),
//! where dropping a falsified value shows up as an invalid free, and aliasing violations are caught.

use std::panic::{catch_unwind, AssertUnwindSafe};

use columnation::*;
use smallvec::{smallvec, SmallVec};

/// A type whose region panics on `copy` when asked to, and on `clear` when asked to.
#[derive(Clone, Debug, PartialEq, Eq)]
struct Bomb { copy: bool, clear: bool }

#[derive(Default)]
struct BombRegion { armed: bool }

impl Region for BombRegion {
    type Item = Bomb;
    unsafe fn copy(&mut self, item: &Bomb) -> Bomb {
        if item.copy { panic!("Bomb::copy"); }
        self.armed |= item.clear;
        item.clone()
    }
    fn clear(&mut self) {
        if std::mem::take(&mut self.armed) { panic!("Bomb::clear"); }
    }
    fn reserve_items<'a, I>(&mut self, _items: I) where Self: 'a, I: Iterator<Item=&'a Bomb>+Clone { }
    fn reserve_regions<'a, I>(&mut self, _regions: I) where Self: 'a, I: Iterator<Item=&'a Self>+Clone { }
    fn heap_size(&self, _callback: impl FnMut(usize, usize)) { }
}

impl Columnation for Bomb { type InnerRegion = BombRegion; }

const SAFE: Bomb = Bomb { copy: false, clear: false };
const COPY_PANICS: Bomb = Bomb { copy: true, clear: false };
const CLEAR_PANICS: Bomb = Bomb { copy: false, clear: true };

/// Values handed out for earlier records must remain readable after later copies.
#[test]
fn earlier_records_remain_valid() {
    let mut stack = ColumnStack::<String>::default();
    let strings = ["a", "b", "c", "d", "e"].map(String::from);
    for s in strings.iter() { stack.copy(s); }
    assert_eq!(&stack[..], &strings[..]);

    let mut stack = ColumnStack::<Vec<Vec<u8>>>::default();
    let record = vec![vec![1u8, 2], vec![3], vec![4, 5, 6]];
    for _ in 0 .. 3 { stack.copy(&record); }
    for element in stack.iter() { assert_eq!(element, &record); }
}

/// A panic copying a later tuple field must not drop falsified earlier fields.
#[test]
fn panic_in_tuple_copy() {
    let mut stack = ColumnStack::<(String, Bomb)>::default();
    stack.copy(&("aaaa".to_string(), SAFE));
    stack.copy(&("bbbb".to_string(), SAFE));
    let result = catch_unwind(AssertUnwindSafe(|| stack.copy(&("cccc".to_string(), COPY_PANICS))));
    assert!(result.is_err());
    assert_eq!(stack.len(), 2);
    assert_eq!(stack[1].0, "bbbb");
}

/// As `panic_in_tuple_copy`, but through `copy_destructured`.
#[test]
fn panic_in_tuple_copy_destructured() {
    let mut stack = ColumnStack::<(String, Bomb)>::default();
    stack.copy_destructured(&"aaaa".to_string(), &SAFE);
    stack.copy_destructured(&"bbbb".to_string(), &SAFE);
    let result = catch_unwind(AssertUnwindSafe(|| stack.copy_destructured(&"cccc".to_string(), &COPY_PANICS)));
    assert!(result.is_err());
    assert_eq!(stack.len(), 2);
}

/// A panic copying a later element of an inline `SmallVec` must not drop falsified earlier elements.
#[test]
fn panic_in_smallvec_inline_copy() {
    let mut stack = ColumnStack::<SmallVec<[Result<String, Bomb>; 4]>>::default();
    stack.copy(&smallvec![Ok("aaaa".to_string())]);
    let record: SmallVec<[Result<String, Bomb>; 4]> = smallvec![Ok("bbbb".to_string()), Err(COPY_PANICS)];
    let result = catch_unwind(AssertUnwindSafe(|| stack.copy(&record)));
    assert!(result.is_err());
    assert_eq!(stack.len(), 1);
}

/// A panic in one region's `clear` (during drop) must not cause sibling regions to drop falsified values.
#[test]
fn panic_in_clear_during_drop() {
    let mut stack = ColumnStack::<(Bomb, Vec<String>)>::default();
    stack.copy(&(SAFE, vec!["aaaa".to_string(), "bbbb".to_string()]));
    stack.copy(&(CLEAR_PANICS, vec!["cccc".to_string()]));
    let result = catch_unwind(AssertUnwindSafe(|| drop(stack)));
    assert!(result.is_err());
}

/// Uses a region directly, following the documented contract of `Region::copy`:
/// the result is only used to reference its contents, and is never dropped.
#[test]
fn region_dropped_without_clear() {
    let mut region = <Vec<String> as Columnation>::InnerRegion::default();
    let record = vec!["aaaa".to_string(), "bbbb".to_string()];
    let copied = unsafe { region.copy(&record) };
    assert_eq!(copied, record);
    std::mem::forget(copied);
    drop(region);
}

/// `StableRegion` promises items do not move; that must hold even if `ExactSizeIterator` lies.
#[test]
fn stable_region_trusts_exact_size() {
    struct Liar(u64);
    impl Iterator for Liar {
        type Item = u64;
        fn next(&mut self) -> Option<u64> { if self.0 > 0 { self.0 -= 1; Some(self.0) } else { None } }
        fn size_hint(&self) -> (usize, Option<usize>) { (0, Some(0)) }
    }
    impl ExactSizeIterator for Liar { }

    let mut region = StableRegion::<u64>::default();
    let first: *const u64 = region.copy_iter(std::iter::once(17)).as_ptr();
    region.copy_iter(Liar(1000));
    // The documented guarantee: `first` still points at the item we inserted.
    assert_eq!(unsafe { *first }, 17);
}
