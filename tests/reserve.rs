//! Tests that `reserve_items` and `reserve_regions` reserve exactly what copying requires.

use columnation::*;
use smallvec::SmallVec;

/// The number of allocations, bytes used, and bytes of capacity of a stack's regions.
fn region_stats<T: Columnation>(stack: &ColumnStack<T>) -> (usize, usize, usize) {
    let (mut allocations, mut used, mut capacity) = (0, 0, 0);
    let mut first = true;
    stack.heap_size(|u, c| {
        // The first callback reports the stack's own `local` vector, which we skip.
        if std::mem::take(&mut first) { return; }
        if c > 0 {
            allocations += 1;
            used += u;
            capacity += c;
        }
    });
    (allocations, used, capacity)
}

/// Asserts that copying `stack` into an empty stack after reserving neither grows nor wastes capacity.
fn assert_exact<T: Columnation>(name: &str, reserved: (usize, usize, usize), stack: &ColumnStack<T>) {
    let (allocations, used, capacity) = region_stats(stack);
    assert_eq!((allocations, capacity), (reserved.0, reserved.2), "{}: regions grew after reserving", name);
    assert_eq!(used, capacity, "{}: reserved unused capacity", name);
}

fn test_reserve<T: Columnation>(name: &str, record: T) {
    let count = 100;

    let mut stack = ColumnStack::<T>::default();
    stack.reserve_items(std::iter::repeat(&record).take(count));
    let reserved = region_stats(&stack);
    for _ in 0 .. count { stack.copy(&record); }
    assert_exact(&format!("{} (reserve_items)", name), reserved, &stack);

    let mut copy = ColumnStack::<T>::default();
    copy.reserve_regions(std::iter::once(&stack));
    let reserved = region_stats(&copy);
    for item in stack.iter() { copy.copy(item); }
    assert_exact(&format!("{} (reserve_regions)", name), reserved, &copy);
}

fn string() -> String { "grawwwwrr!".to_string() }

#[test] fn reserve_string() { test_reserve("String", string()); }
#[test] fn reserve_tuple() { test_reserve("(u64, String)", (0u64, string())); }
#[test] fn reserve_option() { test_reserve("Option<String>", Some(string())); }
#[test] fn reserve_vec() { test_reserve("Vec<u64>", vec![0u64; 100]); }
#[test] fn reserve_vec_empty() { test_reserve("Vec<u64> (empty)", Vec::<u64>::new()); }
#[test] fn reserve_vec_string() { test_reserve("Vec<String>", vec![string(); 10]); }
#[test] fn reserve_vec_vec() { test_reserve("Vec<Vec<u8>>", vec![vec![0u8; 3]; 10]); }
#[test] fn reserve_smallvec_inline() { test_reserve("SmallVec<[u64; 2]> (inline)", SmallVec::<[u64; 2]>::from_slice(&[1])); }
#[test] fn reserve_smallvec_spilled() { test_reserve("SmallVec<[u64; 2]> (spilled)", SmallVec::<[u64; 2]>::from_slice(&[1; 8])); }
#[test] fn reserve_smallvec_string_inline() { test_reserve("SmallVec<[String; 2]> (inline)", SmallVec::<[String; 2]>::from_vec(vec![string(); 2])); }
#[test] fn reserve_smallvec_string_spilled() { test_reserve("SmallVec<[String; 2]> (spilled)", SmallVec::<[String; 2]>::from_vec(vec![string(); 3])); }
#[test]
fn reserve_smallvec_string_short() {
    // Spilled in the source, but short enough to copy inline.
    let mut record = SmallVec::<[String; 2]>::with_capacity(4);
    record.push(string());
    assert!(record.spilled());
    test_reserve("SmallVec<[String; 2]> (short)", record);
}

/// `ColumnStack::reserve_items` should not require the stack to be borrowed as long as the items.
#[test]
fn reserve_items_lifetime() {
    fn reserve<'a, T: Columnation + 'a, I: Iterator<Item=&'a T> + Clone>(stack: &mut ColumnStack<T>, items: I) {
        stack.reserve_items(items);
    }
    let records = vec![string(); 10];
    let mut stack = ColumnStack::<String>::default();
    reserve(&mut stack, records.iter());
    for record in records.iter() { stack.copy(record); }
    assert_eq!(&stack[..], &records[..]);
}

/// Reserving for many long vectors of zero-sized types should not overflow.
#[test]
fn reserve_zero_sized() {
    // Constructed with `set_len`, as older toolchains build `vec![(); usize::MAX]` one element at a time.
    let mut record = Vec::<()>::new();
    // Unsafety justified in that zero-sized elements need no space or initialization.
    unsafe { record.set_len(usize::MAX); }
    let mut stack = ColumnStack::<Vec<()>>::default();
    stack.reserve_items(std::iter::repeat(&record).take(2));
}
