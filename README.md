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
assert_eq!(&my_region[..], &my_data[..]);
```

## Measurements

I took various types of records, generally containing a thousand allocations or so, and either `copy` or `clone` them in to a container 1024 times, just as above. Here are the benchmark times that Rust's `cargo bench` tool provides, where `_clone` is cloning into a vector, and `_copy` is copying into a region-backed container.

```
running 16 tests
test empty_clone      ... bench:       5,453 ns/iter (+/- 199)
test empty_copy       ... bench:      11,042 ns/iter (+/- 134)
test string10_clone   ... bench: 166,110,167 ns/iter (+/- 80,078,857)
test string10_copy    ... bench:   9,157,063 ns/iter (+/- 949,290)
test string20_clone   ... bench:  87,231,969 ns/iter (+/- 12,498,375)
test string20_copy    ... bench:   5,000,124 ns/iter (+/- 464,245)
test u32x2_clone      ... bench:   2,278,221 ns/iter (+/- 165,168)
test u32x2_copy       ... bench:     741,297 ns/iter (+/- 196,473)
test u64_clone        ... bench:   2,353,318 ns/iter (+/- 852,005)
test u64_copy         ... bench:     801,995 ns/iter (+/- 24,183)
test u8_u64_clone     ... bench:   2,322,168 ns/iter (+/- 206,600)
test u8_u64_copy      ... bench:     801,955 ns/iter (+/- 22,525)
test vec_u_s_clone    ... bench: 183,448,971 ns/iter (+/- 51,349,339)
test vec_u_s_copy     ... bench:  10,999,465 ns/iter (+/- 5,863,004)
test vec_u_vn_s_clone ... bench: 198,654,657 ns/iter (+/- 43,369,880)
test vec_u_vn_s_copy  ... bench:  21,096,008 ns/iter (+/- 21,597,098)
```
In each case, other than the intentionally trivial `empty` case, the `_copy` version is markedly faster than the `_clone` version. This makes some sense, as we are able to re-use all of the allocations across runs in the `_copy` case and only the vector's spine in the `_clone` case (we could attempt more complicated buffer pooling, but we haven't done that here).

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