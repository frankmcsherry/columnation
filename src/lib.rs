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
///
/// This type will ensure that absorbed data remain valid as long as the
/// instance itself is valid. Responsible users will couple the lifetime
/// of this instance with that of *all* instances it returns from `copy`.
pub trait Region : Default {
    /// The type of item the region contains.
    type Item;
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
    unsafe fn copy(&mut self, item: &Self::Item) -> Self::Item;
    /// Retain allocations but discard their contents.
    ///
    /// The elements in the region do not actually own resources, and
    /// their normal `Drop` implementations will not be called. This method
    /// should only be called after all instances returned by `copy` have
    /// been disposed of, as this method may invalidate their contents.
    fn clear(&mut self);
}

/// A vacuous region that just clones items.
pub struct CloneRegion<T> {
    phantom: std::marker::PhantomData<T>,
}

impl<T> Default for CloneRegion<T> {
    fn default() -> Self {
        Self { phantom: std::marker::PhantomData }
    }
}

// Any type that implements clone can use a non-region that just clones items.
impl<T: Clone> Region for CloneRegion<T> {
    type Item = T;
    #[inline(always)]
    unsafe fn copy(&mut self, item: &Self::Item) -> Self::Item {
        item.clone()
    }
    #[inline(always)]
    fn clear(&mut self) { }
}


/// A region allocator which holds items at stable memory locations.
///
/// Items once inserted will not me moved, and their locations in memory
/// can be relied on by others, until the region is cleared.
///
/// This type accepts owned data, rather than references, and does not
/// itself intent to implement `Region`. Rather, it is a useful building
/// block for other less-safe code that wants allocated data to remain at
/// fixed memory locations.
pub struct StableRegion<T> {
    /// The active allocation into which we are writing.
    local: Vec<T>,
    /// All previously active allocations.
    stash: Vec<Vec<T>>,
}

// Manually implement `Default` as `T` may not implement it.
impl<T> Default for StableRegion<T> {
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
            // Unsafety justified in that setting the length to zero exposes
            // no invalid data.
            self.local.set_len(0);
            // Release allocations in `stash` without dropping their elements.
            for mut buffer in self.stash.drain(..) {
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
            let next_len = (self.local.capacity() + 1).next_power_of_two();
            let new_local = Vec::with_capacity(std::cmp::max(items.len(), next_len));
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
            let next_len = (self.local.capacity() + 1).next_power_of_two();
            let new_local = Vec::with_capacity(std::cmp::max(items.len(), next_len));
            self.stash.push(std::mem::replace(&mut self.local, new_local));
        }

        let initial_len = self.local.len();
        self.local.extend_from_slice(items);
        &mut self.local[initial_len ..]
    }
}


/// A type that can be stored in a columnar region.
///
/// This trait exists only to allow types to name the columnar region
/// that should be used
pub trait Columnation: Sized {
    /// The type of region capable of absorbing allocations owned by
    /// the `Self` type. Note: not allocations of `Self`, but of the
    /// things that it owns.
    type InnerRegion: Region<Item = Self>;
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
    /// Construct a [ColumnStack], reserving space for `capacity` elements
    ///
    /// Note that the associated region is not initialized to a specific capacity
    /// because we can't generally know how much space would be required.
    pub fn with_capacity(capacity: usize) -> Self {
        Self {
            local: Vec::with_capacity(capacity),
            inner: T::InnerRegion::default(),
        }
    }

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
            // Unsafety justified in that setting the length to zero exposes
            // no invalid data.
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
        unsafe {
            // Unsafety justified in that `write_position` is no greater than
            // `self.local.len()` and so this exposes no invalid data.
            self.local.set_len(write_position);
        }
    }
}

impl<T: Columnation> std::ops::Deref for ColumnStack<T> {
    type Target = [T];
    #[inline(always)]
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

impl<'a, T: Columnation + 'a> Extend<&'a T> for ColumnStack<T> {
    fn extend<I: IntoIterator<Item=&'a T>>(&mut self, iter: I) {
        for element in iter {
            self.copy(element)
        }
    }
}

impl<'a, T: Columnation + 'a> std::iter::FromIterator<&'a T> for ColumnStack<T> {
    fn from_iter<I: IntoIterator<Item=&'a T>>(iter: I) -> Self {
        let iter = iter.into_iter();
        let mut c = ColumnStack::<T>::with_capacity(iter.size_hint().0);
        c.extend(iter);
        c
    }
}

impl<T: Columnation + PartialEq> PartialEq for ColumnStack<T> {
    fn eq(&self, other: &Self) -> bool {
        PartialEq::eq(&self[..], &other[..])
    }
}

impl<T: Columnation + Eq> Eq for ColumnStack<T> {}

impl<T: Columnation + std::fmt::Debug> std::fmt::Debug for ColumnStack<T> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        (&self[..]).fmt(f)
    }
}

impl<T: Columnation> Clone for ColumnStack<T> {
    fn clone(&self) -> Self {
        let mut new: Self = Default::default();
        for item in &self[..] {
            new.copy(item);
        }
        new
    }

    fn clone_from(&mut self, source: &Self) {
        self.clear();
        for item in &source[..] {
            self.copy(item);
        }
    }
}

mod implementations {

    use super::{Region, CloneRegion, StableRegion, Columnation, ColumnStack};

    // Implementations for types whose `clone()` suffices for the region.
    macro_rules! implement_columnation {
        ($index_type:ty) => (
            impl Columnation for $index_type {
                type InnerRegion = CloneRegion<$index_type>;
            }
        )
    }
    implement_columnation!(());
    implement_columnation!(bool);
    implement_columnation!(char);

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

    implement_columnation!(f32);
    implement_columnation!(f64);

    implement_columnation!(std::num::Wrapping<i8>);
    implement_columnation!(std::num::Wrapping<i16>);
    implement_columnation!(std::num::Wrapping<i32>);
    implement_columnation!(std::num::Wrapping<i64>);
    implement_columnation!(std::num::Wrapping<i128>);
    implement_columnation!(std::num::Wrapping<isize>);

    implement_columnation!(std::time::Duration);

    /// Implementations for `Option<T: Columnation>`.
    pub mod option {

        use super::{Columnation, Region};

        #[derive(Default)]
        pub struct OptionRegion<R: Region> {
            region: R,
        }

        impl<R: Region> Region for OptionRegion<R> {
            type Item = Option<R::Item>;
            #[inline(always)]
            unsafe fn copy(&mut self, item: &Self::Item) -> Self::Item {
                item.as_ref().map(|inner| self.region.copy(inner))
            }
            #[inline(always)]
            fn clear(&mut self) {
                self.region.clear();
            }
        }

        impl<T: Columnation> Columnation for Option<T> {
            type InnerRegion = OptionRegion<T::InnerRegion>;
        }
    }

    /// Implementations for `Result<T: Columnation, E: Columnation>`.
    pub mod result {

        use super::{Columnation, Region};

        #[derive(Default)]
        pub struct ResultRegion<R1: Region, R2: Region> {
            region1: R1,
            region2: R2,
        }


        impl<R1: Region, R2: Region> Region for ResultRegion<R1, R2> {
            type Item = Result<R1::Item, R2::Item>;
            #[inline(always)]
            unsafe fn copy(&mut self, item: &Self::Item) -> Self::Item {
                match item {
                    Ok(item) => { Ok(self.region1.copy(item)) },
                    Err(item) => { Err(self.region2.copy(item)) },
                }
            }
            #[inline(always)]
            fn clear(&mut self) {
                self.region1.clear();
                self.region2.clear();
            }
        }

        impl<T: Columnation, E: Columnation> Columnation for Result<T, E> {
            type InnerRegion = ResultRegion<T::InnerRegion, E::InnerRegion>;
        }
    }

    /// Implementations for `Vec<T: Columnation>`.
    pub mod vec {

        use super::{Columnation, Region, StableRegion};

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

        impl<T: Columnation> Region for VecRegion<T> {
            type Item = Vec<T>;
            #[inline]
            fn clear(&mut self) {
                self.region.clear();
                self.inner.clear();
            }
            #[inline(always)]
            unsafe fn copy(&mut self, item: &Self::Item) -> Self::Item {
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

        use super::{Columnation, Region, StableRegion};

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

        impl Region for StringStack {
            type Item = String;
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

    /// Implementation for tuples.
    pub mod tuple {

        use super::{Columnation, ColumnStack, Region};

        use paste::paste;

        /// The macro creates the region implementation for tuples
        macro_rules! tuple_columnation {
            ( $($name:ident)+) => ( paste! {
                impl<$($name: Columnation),*> Columnation for ($($name,)*) {
                    type InnerRegion = [<Tuple $($name)* Region >]<$($name::InnerRegion,)*>;
                }

                #[allow(non_snake_case)]
                #[derive(Default)]
                pub struct [<Tuple $($name)* Region >]<$($name: Region),*> {
                    $([<region $name>]: $name),*
                }

                #[allow(non_snake_case)]
                impl<$($name: Region),*> [<Tuple $($name)* Region>]<$($name),*> {
                    #[allow(clippy::too_many_arguments)]
                    #[inline] pub unsafe fn copy_destructured(&mut self, $([<r $name>]: &$name::Item),*) -> <[<Tuple $($name)* Region>]<$($name),*> as Region>::Item {
                        (
                            $(self.[<region $name>].copy(&[<r $name>]),)*
                        )
                    }
                }

                #[allow(non_snake_case)]
                impl<$($name: Region),*> Region for [<Tuple $($name)* Region>]<$($name),*> {
                    type Item = ($($name::Item,)*);
                    #[inline]
                    fn clear(&mut self) {
                        $(self.[<region $name>].clear());*
                    }
                    #[inline] unsafe fn copy(&mut self, item: &Self::Item) -> Self::Item {
                        let ($(ref $name,)*) = *item;
                        (
                            $(self.[<region $name>].copy($name),)*
                        )
                    }
                }
                }
                tuple_column_stack!(ColumnStack, $($name)*);
            );
        }

        /// The macro creates the `copy_destructured` implementation for a custom column stack
        /// with a single generic parameter characterizing the type `T` it stores.
        /// It assumes there are two fields on `self`:
        /// * `local`: A type supporting `push(T)`, e.g, `Vec`.
        /// * `inner`: A region of type `T`.
        /// We're exporting this macro so custom `ColumnStack` implementations can benefit from it.
        #[macro_export]
        macro_rules! tuple_column_stack {
            ( $type:ident, $($name:ident)+) => (
                #[allow(non_snake_case)]
                impl<$($name: Columnation),*> $type<($($name,)*)> {
                    /// Copies a destructured tuple into this column stack.
                    ///
                    /// This serves situations where a tuple should be constructed from its constituents but not
                    /// not all elements are available as owned data.
                    ///
                    /// The element can be read by indexing
                    pub fn copy_destructured(&mut self, $($name: &$name,)*) {
                        unsafe {
                            self.local.push(self.inner.copy_destructured($($name,)*));
                        }
                    }
                }
            );
        }

        tuple_columnation!(A);
        tuple_columnation!(A B);
        tuple_columnation!(A B C);
        tuple_columnation!(A B C D);
        tuple_columnation!(A B C D E);
        tuple_columnation!(A B C D E F);
        tuple_columnation!(A B C D E F G);
        tuple_columnation!(A B C D E F G H);
        tuple_columnation!(A B C D E F G H I);
        tuple_columnation!(A B C D E F G H I J);
        tuple_columnation!(A B C D E F G H I J K);
        tuple_columnation!(A B C D E F G H I J K L);
        tuple_columnation!(A B C D E F G H I J K L M);
        tuple_columnation!(A B C D E F G H I J K L M N);
        tuple_columnation!(A B C D E F G H I J K L M N O);
        tuple_columnation!(A B C D E F G H I J K L M N O P);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R S);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R S T);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R S T U);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R S T U V);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R S T U V W);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R S T U V W X);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R S T U V W X Y);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R S T U V W X Y Z);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R S T U V W X Y Z AA);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R S T U V W X Y Z AA AB);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R S T U V W X Y Z AA AB AC);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R S T U V W X Y Z AA AB AC AD);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R S T U V W X Y Z AA AB AC AD AE);
        tuple_columnation!(A B C D E F G H I J K L M N O P Q R S T U V W X Y Z AA AB AC AD AE AF);
    }
}
