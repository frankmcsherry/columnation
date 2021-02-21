# Columnation
An experimental columnar arena

Columnation borrows its name from [Abomonation](https://github.com/TimelyDataflow/abomonation), a Rust serialization framework that is very fast and very unsafe. Among Abomonation's unsafeties, it hosts typed data backed by `[u8]` slices, which causes anxiety for folks who worry about alignment, the visibility of padding bytes, and likely other things I don't yet know about.

Columnation is slightly better, in that it only maintains typed allocations `Vec<T>`, and does not invoke `mem::transmute` to change types. This does not mean it is *safe*, just that there are fewer places in the code that are likely to be unsafe once Rust's unsafety story shakes out.

Instead, Columnation works by consolidating the owned allocations that types might have (*e.g.* a `String`, or a `Vec<T>` for other types `T`) into relatively fewer and larger allocations, and rewriting pointers in these types to point in to the larger allocations. This makes the types unsuitable for any use other than through an immutable reference. It also means that Columnation is plenty unsafe, and just an experiment for the moment.

## Example use

Some types implement `Columnation`, a Rust trait that has no methods but allows one to instantiate a `ColumnStack`, into which you can copy instances of the type. The instances are then available through the stack's `Deref` implementation, which presents the stack as a `&[Type]`. Indexed elements present as `&Type`, and in principle you can do all the usual things with those references, despite not actually having an actual `Type` backing them.

```rust
// Some data are suitable for translation into columnar form.
let my_data = vec![vec![(0u64, vec![(); 1 << 40], format!("grawwwwrr!")); 32]; 32];

// Such data can be copied in to a columnar region.
let mut my_region = ColumnStack::default();
for _ in 0 .. 1024 {
    my_region.copy(&my_data);
}

// The copying above is substantially faster than cloning the
// data, 21ms versus 198ms, when cloned like so:
let mut my_vec = Vec::with_capacity(1024);
for _ in 0 .. 1024 {
    my_vec.push(my_data.clone());
}

// At this point, `my_region` has just tens of allocations,
// despite presenting as if a thousand records which would
// normally have a thousand allocations behind each of them.
assert_eq!(&my_region[..], &my_vec[..]);
```

## Measurements

I took various types of records, generally containing a thousand allocations or so, and either `copy` or `clone` them in to a container 1024 times, just as above. Here are the benchmark times that Rust's `cargo bench` tool provides, where `_clone` is cloning into a vector, and `_copy` is copying into a region-backed container.

```
running 16 tests
test empty_clone      ... bench:         835 ns/iter (+/- 114)
test empty_copy       ... bench:       3,103 ns/iter (+/- 346)
test string10_clone   ... bench: 108,914,118 ns/iter (+/- 7,864,343)
test string10_copy    ... bench:   4,654,702 ns/iter (+/- 312,409)
test string20_clone   ... bench:  59,302,789 ns/iter (+/- 8,014,183)
test string20_copy    ... bench:   2,818,970 ns/iter (+/- 191,415)
test u32x2_clone      ... bench:   1,920,494 ns/iter (+/- 198,815)
test u32x2_copy       ... bench:     282,235 ns/iter (+/- 40,851)
test u64_clone        ... bench:   1,951,842 ns/iter (+/- 129,288)
test u64_copy         ... bench:     234,412 ns/iter (+/- 25,186)
test u8_u64_clone     ... bench:   1,931,056 ns/iter (+/- 162,882)
test u8_u64_copy      ... bench:     266,326 ns/iter (+/- 35,203)
test vec_u_s_clone    ... bench: 120,642,691 ns/iter (+/- 9,124,488)
test vec_u_s_copy     ... bench:   5,801,229 ns/iter (+/- 522,024)
test vec_u_vn_s_clone ... bench: 134,171,625 ns/iter (+/- 18,599,137)
test vec_u_vn_s_copy  ... bench:   8,580,739 ns/iter (+/- 451,180)
```
In each case, other than the intentionally trivial `empty` case, the `_copy` version is markedly faster than the `_clone` version. This makes some sense, as we are able to re-use all of the allocations across runs in the `_copy` case and only the vector's spine in the `_clone` case (we could attempt more complicated buffer pooling, but we haven't done that here).

The `empty` case has an interesting story. When consolidating the multiple `Vec<()>` allocations into one `Vec<()>`, we introduce the cost of maitnaining the length and capacity of that vector. It should be "unbounded" with a zero-sized type, but in fact we need to verify that it does not read `usize::MAX`. The `empty_clone` case does not need to do this, and optimizes to a `memcpy`, whereas the `empty_copy` case must check the capacity between each insertion.

## Description

A type like `Vec<String>` would be encoded in Columnation using memory that roughly resembles
```rust
struct Roughly {
    /// Where each `Vec<String>` record is put.
    records: Vec<Vec<String>>,
    /// All of the `String` in all of the records.
    strings: Vec<String>
    /// All of the bytes in all of the strings in all of the records.
    bytes: Vec<u8>,
}
```
In fact, each member is a sequence of allocations, because we will not be able to re-size any of our allocations (without substantial work) and we instead create new geometrically increasing allocations. The struct has a special `Drop` implementation that releases the three bundles of allocations, but does not recursively call the `Drop` implementation of their members (to avoid attempting to release the pointers to the interior of the allocations, which we know are not valid).

More general types need to describe how to safely store owned allocations they may have. The `Columnation` trait they implement allows them to specify an associated type, `InnerRegion`, which is what absorbs allocations owned by the type.

## Relation to Abomonation

The intent of Columnation is similar to Abomonation, to present immutable references to Rust types without requiring the existence of actual owned types, but their memory layouts are different.

Whereas Abomonation serializes each record contiguously, Columnation "serializes" the allocations behind each record separately into distinct statically typed buffers. Abomonation has locality-of-reference advantages when you want to investigate an entire record, and Columnation has memory compactness advantages when you often need only subsets of the allocations backing records.

## Safety

Part of the goal of pushing the code is to get some eyes on it. It could be safe, but I am not clear on the pointer aliasing rules required for safety (nor where they are recorded). Also, there could totally be bugs, but I should fix those.