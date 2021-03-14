//! An unsafe columnar arena for owned data.
//!
//! This library contains types and traits that allow one to collect
//! types with owned data but is backed by relatively few allocations.
//! The catch is that collected instances can only be used by reference,
//! as they are not valid owned data (their pointers do not point to
//! allocations that can be returned to the allocator).
//!
//! # Safety
//!
//! This crate is wildly unsafe, on account of it uses the `unsafe`
//! keyword and Rust's safety is not yet clearly enough specified
//! for me to make any stronger statements than that.

/// A type that can absorb owned data from type `T`.
pub trait ColumnarRegion<T> : Default {
    /// Add a new element to the region.
    ///
    /// The argument will be copied in to the region and returned as an
    /// owned instance. It is unsafe to unwrap and then drop the result.
    ///
    /// # Safety
    ///
    /// It is unsafe to use the result in any way other than to reference
    /// its contents, and then only for the lifetime of the columnar region.
    /// Correct uses of this method are very likely exclusive to this crate.
    unsafe fn copy(&mut self, item: &T) -> T;
    /// Retain allocations but discard their contents.
    ///
    /// The elements in the region do not actually own resources, and
    /// their normal `Drop` implementations will not be called.
    fn clear(&mut self);
}

/// A type that can be stored in a columnar region.
pub trait Columnation: Sized {
    /// The type of region capable of absorbing allocations owned by
    /// the `Self` type. Note: not allocations of `Self`, but of the
    /// things that it owns.
    type InnerRegion: ColumnarRegion<Self>;
}


/// An append-only vector that store records as columns.
///
/// This container maintains elements that might conventionally own
/// memory allocations, but instead the pointers to those allocations
/// reference larger regions of memory shared with multiple instances
/// of the type. Elements can be retrieved as references, and care is
/// taken when this type is dropped to ensure that the correct memory
/// is returned (rather than the incorrect memory, from running the
/// elements `Drop` implementations).
pub struct ColumnStack<T: Columnation> {
    local: Vec<T>,
    inner: T::InnerRegion,
}

impl<T: Columnation> ColumnStack<T> {
    /// Copies an element in to the region.
    ///
    /// The element can be read by indexing
    pub fn copy(&mut self, item: &T) {
        // TODO: Some types `T` should just be cloned.
        // E.g. types that are `Copy` or vecs of ZSTs.
        unsafe {
            self.local.push(self.inner.copy(item));
        }
    }
    /// Empties the collection.
    pub fn clear(&mut self) {
        unsafe {
            self.local.set_len(0);
            self.inner.clear();
        }
    }
    /// Retain elements that pass a predicate, from a specified offset.
    ///
    /// This method may or may not reclaim memory in the inner region.
    pub fn retain_from<P: FnMut(&T)->bool>(&mut self, index: usize, mut predicate: P) {
        let mut write_position = index;
        for position in index .. self.local.len() {
            if predicate(&self[position]) {
                // TODO: compact the inner region and update pointers.
                self.local.swap(position, write_position);
                write_position += 1;
            }
        }
        self.local.truncate(write_position);
    }
}

impl<T: Columnation> std::ops::Deref for ColumnStack<T> {
    type Target = [T];
    fn deref(&self) -> &Self::Target {
        &self.local[..]
    }
}

impl<T: Columnation> Drop for ColumnStack<T> {
    fn drop(&mut self) {
        self.clear();
    }
}

impl<T: Columnation> Default for ColumnStack<T> {
    fn default() -> Self {
        Self {
            local: Vec::new(),
            inner: T::InnerRegion::default(),
        }
    }
}



/// A region allocator which holds items at stable memory locations.
///
/// Items once inserted will not me moved, and their locations in memory
/// can be relied on by others, until the region is cleared.
pub struct StableRegion<T> {
    /// The active allocation into which we are writing.
    local: Vec<T>,
    /// All previously active allocations.
    stash: Vec<Vec<T>>,
}

// Manually implement `Default` as `T` may not implement it.
impl<T: Columnation> Default for StableRegion<T> {
    fn default() -> Self {
        Self {
            local: Vec::new(),
            stash: Vec::new(),
        }
    }
}

impl<T> StableRegion<T> {
    /// Clears the contents without dropping any elements.
    #[inline]
    pub fn clear(&mut self) {
        unsafe {
            self.local.set_len(0);
            for buffer in self.stash.iter_mut() {
                buffer.set_len(0);
            }
        }
    }
    /// Copies an iterator of items into the region.
    #[inline]
    pub fn copy_iter<I>(&mut self, items: I) -> &mut [T]
    where
        I: Iterator<Item = T> + std::iter::ExactSizeIterator,
    {
        // Check if `item` fits into `self.local` without reallocation.
        // If not, stash `self.local` and increase the allocation.
        if items.len() > self.local.capacity() - self.local.len() {
            // Increase allocated capacity in powers of two.
            // We could choose a different rule here if we wanted to be
            // more conservative with memory (e.g. page size allocations).
            let len = self.stash.len();
            let new_local = Vec::with_capacity(std::cmp::max(items.len(), 1 << len));
            self.stash.push(std::mem::replace(&mut self.local, new_local));
        }

        let initial_len = self.local.len();
        self.local.extend(items);
        &mut self.local[initial_len ..]
    }
    /// Copies a slice of cloneable items into the region.
    #[inline]
    pub fn copy_slice(&mut self, items: &[T]) -> &mut [T]
    where
        T: Clone,
    {
        // Check if `item` fits into `self.local` without reallocation.
        // If not, stash `self.local` and increase the allocation.
        if items.len() > self.local.capacity() - self.local.len() {
            // Increase allocated capacity in powers of two.
            // We could choose a different rule here if we wanted to be
            // more conservative with memory (e.g. page size allocations).
            let len = self.stash.len();
            let new_local = Vec::with_capacity(std::cmp::max(items.len(), 1 << len));
            self.stash.push(std::mem::replace(&mut self.local, new_local));
        }

        let initial_len = self.local.len();
        self.local.extend_from_slice(items);
        &mut self.local[initial_len ..]
    }
}


mod implementations {

    use super::{Columnation, ColumnarRegion, StableRegion};

    // Implementations for non-owning types, whose implementations can
    // simply be empty. This macro should only be used for types whose
    // bit-wise copy is sufficient to clone the record.
    macro_rules! implement_columnation {
        ($index_type:ty) => (
            impl ColumnarRegion<$index_type> for () {
                #[inline(always)]
                unsafe fn copy(&mut self, item: &$index_type) -> $index_type {
                    *item
                }
                #[inline(always)]
                fn clear(&mut self) { }
            }
            impl Columnation for $index_type {
                type InnerRegion = ();
            }
        )
    }
    implement_columnation!(());
    implement_columnation!(bool);

    implement_columnation!(u8);
    implement_columnation!(u16);
    implement_columnation!(u32);
    implement_columnation!(u64);
    implement_columnation!(u128);
    implement_columnation!(usize);

    implement_columnation!(i8);
    implement_columnation!(i16);
    implement_columnation!(i32);
    implement_columnation!(i64);
    implement_columnation!(i128);
    implement_columnation!(isize);

    /// Implementations for `Option<T: Columnation>`.
    pub mod option {

        use super::{Columnation, ColumnarRegion};

        impl<T: Columnation> ColumnarRegion<Option<T>> for T::InnerRegion {
            #[inline(always)]
            unsafe fn copy(&mut self, item: &Option<T>) -> Option<T> {
                item.as_ref().map(|inner| <Self as ColumnarRegion<T>>::copy(self, inner))
            }
            #[inline(always)]
            fn clear(&mut self) {
                <Self as ColumnarRegion<T>>::clear(self);
            }
        }

        impl<T: Columnation> Columnation for Option<T> {
            type InnerRegion = T::InnerRegion;
        }
    }

    /// Implementations for `Result<T: Columnation, E: Columnation>`.
    pub mod result {

        use super::{Columnation, ColumnarRegion};

        impl<T: Columnation, E: Columnation> ColumnarRegion<Result<T, E>> for (T::InnerRegion, E::InnerRegion) {
            #[inline(always)]
            unsafe fn copy(&mut self, item: &Result<T, E>) -> Result<T,E> {
                match item {
                    Ok(item) => { Ok(self.0.copy(item)) },
                    Err(item) => { Err(self.1.copy(item)) },
                }
            }
            #[inline(always)]
            fn clear(&mut self) {
                self.0.clear();
                self.1.clear();
            }
        }

        impl<T: Columnation, E: Columnation> Columnation for Result<T, E> {
            type InnerRegion = (T::InnerRegion, E::InnerRegion);
        }
    }

    /// Implementations for `Vec<T: Columnation>`.
    pub mod vec {

        use super::{Columnation, ColumnarRegion, StableRegion};

        /// Region allocation for the contents of `Vec<T>` types.
        ///
        /// Items `T` are stored in stable contiguous memory locations,
        /// and then a `Vec<T>` referencing them is falsified.
        pub struct VecRegion<T: Columnation> {
            /// Region for stable memory locations for `T` items.
            region: StableRegion<T>,
            /// Any inner region allocations.
            inner: T::InnerRegion,
        }

        // Manually implement `Default` as `T` may not implement it.
        impl<T: Columnation> Default for VecRegion<T> {
            fn default() -> Self {
                VecRegion {
                    region: StableRegion::<T>::default(),
                    inner: T::InnerRegion::default(),
                }
            }
        }

        impl<T: Columnation> Columnation for Vec<T> {
            type InnerRegion = VecRegion<T>;
        }

        impl<T: Columnation> ColumnarRegion<Vec<T>> for VecRegion<T> {
            #[inline]
            fn clear(&mut self) {
                self.region.clear();
                self.inner.clear();
            }
            #[inline]
            unsafe fn copy(&mut self, item: &Vec<T>) -> Vec<T> {
                // TODO: Some types `T` should just be cloned, with `copy_slice`.
                // E.g. types that are `Copy` or vecs of ZSTs.
                let inner = &mut self.inner;
                let slice = self.region.copy_iter(item.iter().map(|element| inner.copy(element)));
                Vec::from_raw_parts(slice.as_mut_ptr(), item.len(), item.len())
            }
        }
    }

    /// Implementation for `String`.
    pub mod string {

        use super::{Columnation, ColumnarRegion, StableRegion};

        /// Region allocation for `String` data.
        ///
        /// Content bytes are stored in stable contiguous memory locations,
        /// and then a `String` referencing them is falsified.
        #[derive(Default)]
        pub struct StringStack {
            region: StableRegion<u8>,
        }

        impl Columnation for String {
            type InnerRegion = StringStack;
        }

        impl ColumnarRegion<String> for StringStack {
            #[inline]
            fn clear(&mut self) {
                self.region.clear();
            }
            // Removing `(always)` is a 20% performance regression in
            // the `string10_copy` benchmark.
            #[inline(always)] unsafe fn copy(&mut self, item: &String) -> String {
                let bytes = self.region.copy_slice(item.as_bytes());
                String::from_raw_parts(bytes.as_mut_ptr(), item.len(), item.len())
            }
        }
    }

    /// Implementation for tuples. Macros seemed hard.
    pub mod tuple {

        use super::{Columnation, ColumnarRegion};

        impl<T1: Columnation, T2: Columnation> Columnation for (T1, T2) {
            type InnerRegion = (T1::InnerRegion, T2::InnerRegion);
        }

        impl<T1: Columnation, T2: Columnation> ColumnarRegion<(T1,T2)> for (T1::InnerRegion, T2::InnerRegion) {
            #[inline]
            fn clear(&mut self) {
                self.0.clear();
                self.1.clear();
            }
            #[inline] unsafe fn copy(&mut self, item: &(T1,T2)) -> (T1,T2) {
                (
                    self.0.copy(&item.0),
                    self.1.copy(&item.1),
                )
            }
        }

        impl<T1: Columnation, T2: Columnation, T3: Columnation> Columnation for (T1, T2, T3) {
            type InnerRegion = (T1::InnerRegion, T2::InnerRegion, T3::InnerRegion);
        }

        impl<T1: Columnation, T2: Columnation, T3: Columnation> ColumnarRegion<(T1,T2,T3)> for (T1::InnerRegion, T2::InnerRegion, T3::InnerRegion) {
            #[inline]
            fn clear(&mut self) {
                self.0.clear();
                self.1.clear();
                self.2.clear();
            }
            #[inline] unsafe fn copy(&mut self, item: &(T1,T2,T3)) -> (T1,T2,T3) {
                (
                    self.0.copy(&item.0),
                    self.1.copy(&item.1),
                    self.2.copy(&item.2),
                )
            }
        }
    }
}
