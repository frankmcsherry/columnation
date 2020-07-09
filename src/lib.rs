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
    /// The argument is a valid pointer which can be read, although the
    /// method is required to copy its owned resources in to the region
    /// and then update `item` to reference the new locations.
    unsafe fn copy(&mut self, item: *mut T);
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
        unsafe {
            let mut read = std::ptr::read(item);
            self.inner.copy(&mut read);
            self.local.push(read);
        }
    }
    /// Empties the collection.
    pub fn clear(&mut self) {
        unsafe {
            self.local.set_len(0);
            self.inner.clear();
        }
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
        unsafe {
            self.local.set_len(0);
        }
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


// Implementations for non-owning types, whose implementations can
// simply be empty. This macro should only be used for types whose
// bit-wise copy is sufficient to clone the record.
macro_rules! implement_columnation {
    ($index_type:ty) => (
        impl ColumnarRegion<$index_type> for () {
            #[inline(always)] unsafe fn copy(&mut self, _item: *mut $index_type) { }
            #[inline(always)] fn clear(&mut self) { }
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
        unsafe fn copy(&mut self, item: *mut Option<T>) {
            let mut read = std::ptr::read(item);
            if let Some(item) = &mut read {
                <Self as ColumnarRegion<T>>::copy(self, item);
            }
            std::mem::forget(read);
        }
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
        unsafe fn copy(&mut self, item: *mut Result<T,E>) {
            let mut read = std::ptr::read(item);
            match &mut read {
                Ok(item) => self.0.copy(item),
                Err(item) => self.1.copy(item),
            }
            std::mem::forget(read);
        }
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

    use super::{Columnation, ColumnarRegion};

    pub struct VecRegion<T: Columnation> {
        local: Vec<Vec<T>>,
        inner: T::InnerRegion,
    }

    impl<T: Columnation> Drop for VecRegion<T> {
        fn drop(&mut self) {
            for buffer in self.local.iter_mut() {
                unsafe {
                    buffer.set_len(0);
                }
            }
        }
    }

    impl<T: Columnation> Default for VecRegion<T> {
        fn default() -> Self {
            // We put an empty initial list in place to ensure that
            // calls to `last()` always succeed.
            VecRegion {
                local: vec![Vec::new()],
                inner: T::InnerRegion::default(),
            }
        }
    }

    impl<T: Columnation> Columnation for Vec<T> {
        type InnerRegion = VecRegion<T>;
    }

    impl<T: Columnation> ColumnarRegion<Vec<T>> for VecRegion<T> {
        fn clear(&mut self) {
            for buffer in self.local.iter_mut() {
                unsafe {
                    buffer.set_len(0);
                }
            }
            self.inner.clear();
        }
        #[inline]
        unsafe fn copy(&mut self, item: *mut Vec<T>) {
            // We need to ensure there is an allocation which can
            // absorb all of the elements of `item`, which may mean
            // introducing a new buffer.
            let available =
            self.local
                .last()
                .map(|b| b.capacity() - b.len())
                .unwrap_or(0);

            let read = std::ptr::read(item);
            let item_len = read.len();
            if item_len > available {
                // Increase available length in powers of two.
                // We could choose a different rule here if we
                // wanted to be more conservative with memory.
                let len = self.local.len();
                self.local.push(Vec::with_capacity(std::cmp::max(item_len, 1 << len)));
            }

            let buffer = self.local.last_mut().unwrap();
            let ptr = (buffer.as_ptr() as *mut T).add(buffer.len());
            ptr.copy_from_nonoverlapping(read.as_ptr(), item_len);
            for i in 0 .. item_len {
                self.inner.copy(ptr.add(i))
            }
            buffer.set_len(buffer.len() + item_len);
            std::ptr::write(item, Vec::from_raw_parts(ptr, item_len, item_len));
            std::mem::forget(read);
        }
    }
}

/// Implementation for `String`.
pub mod string {

    use super::{Columnation, ColumnarRegion};

    /// An apparently owning stack which does not drop its elements.
    ///
    /// This stack maintains elements that cannot be dropped using
    /// Rust's traditional drop implementations, as they do not own
    /// the data they reference. Instead, their references are owned
    /// by `inner`, which will be independently deallocated. Mainly,
    /// this means that the `Drop` implementation should zero out
    /// `stack` and carefully release the backing allocation.
    pub struct StringStack {
        local: Vec<Vec<u8>>,
    }

    impl Default for StringStack {
        fn default() -> Self {
            // We put an empty initial list in place to ensure that
            // calls to `last()` always succeed.
            Self {
                local: vec![Vec::new()],
            }
        }
    }

    impl Columnation for String {
        type InnerRegion = StringStack;
    }

    impl ColumnarRegion<String> for StringStack {
        fn clear(&mut self) {
            for buffer in self.local.iter_mut() {
                unsafe {
                    buffer.set_len(0);
                }
            }
         }
        #[inline(always)] unsafe fn copy(&mut self, item: *mut String) {
            // We need to ensure there is an allocation which can
            // absorb all of the elements of `item`, which may mean
            // introducing a new buffer.
            let available =
            self.local
                .last()
                .map(|b| b.capacity() - b.len())
                .unwrap_or(0);

            let read = std::ptr::read(item);
            let item_len = read.len();

            if item_len > available {
                // Increase available length in powers of two.
                // We could choose a different rule here if we
                // wanted to be more conservative with memory.
                let len = self.local.len();
                self.local.push(Vec::with_capacity(std::cmp::max(item_len, 1 << len)));
            }

            let buffer = self.local.last_mut().unwrap();
            let ptr = (buffer.as_ptr() as *mut u8).add(buffer.len());
            ptr.copy_from_nonoverlapping(read.as_ptr(), item_len);
            buffer.set_len(buffer.len() + item_len);
            std::ptr::write(item, String::from_raw_parts(ptr, item_len, item_len));
            std::mem::forget(read);
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
        fn clear(&mut self) {
            self.0.clear();
            self.1.clear();
        }
        #[inline(always)] unsafe fn copy(&mut self, item: *mut (T1,T2)) {
            self.0.copy(&mut ((*item).0) as *mut T1);
            self.1.copy(&mut ((*item).1) as *mut T2);
        }
    }

    impl<T1: Columnation, T2: Columnation, T3: Columnation> Columnation for (T1, T2, T3) {
        type InnerRegion = (T1::InnerRegion, T2::InnerRegion, T3::InnerRegion);
    }

    impl<T1: Columnation, T2: Columnation, T3: Columnation> ColumnarRegion<(T1,T2,T3)> for (T1::InnerRegion, T2::InnerRegion, T3::InnerRegion) {
        fn clear(&mut self) {
            self.0.clear();
            self.1.clear();
            self.2.clear();
        }
        #[inline(always)] unsafe fn copy(&mut self, item: *mut (T1,T2,T3)) {
            self.0.copy(&mut ((*item).0) as *mut T1);
            self.1.copy(&mut ((*item).1) as *mut T2);
            self.2.copy(&mut ((*item).2) as *mut T3);
        }
    }
}
