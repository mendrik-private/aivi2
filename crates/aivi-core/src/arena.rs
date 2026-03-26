use std::{error::Error, fmt, marker::PhantomData, ops::Index};

/// Typed arena index used across typed-core node families.
pub trait ArenaId: Copy + Eq + Ord + fmt::Display + std::hash::Hash {
    fn from_raw(raw: u32) -> Self;
    fn as_raw(self) -> u32;

    fn index(self) -> usize {
        self.as_raw() as usize
    }
}

/// Fallible arena insertion error for node families that exceed the current raw-id width.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ArenaOverflow {
    attempted_len: usize,
}

impl ArenaOverflow {
    pub const fn attempted_len(self) -> usize {
        self.attempted_len
    }
}

impl fmt::Display for ArenaOverflow {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "arena overflow after {} entries; typed-core ids are limited to u32::MAX",
            self.attempted_len
        )
    }
}

impl Error for ArenaOverflow {}

/// Allocate a value into a typed-core [`Arena`], or push an error via a local `arena_overflow`
/// helper and return early from the enclosing function.
///
/// # Requirements
///
/// A function named `arena_overflow(family: &'static str, overflow: ArenaOverflow) -> E` must be
/// in scope at the call site, where `E` is the element type of `$errors`.
///
/// # Variants
///
/// ```ignore
/// // Returns `()` on overflow.
/// alloc_or_diag!(arena, value, "family", errors);
///
/// // Returns `None` on overflow (for Option-returning functions).
/// alloc_or_diag!(arena, value, "family", errors, return None);
///
/// // Propagates with `?` on overflow (for Result-returning functions).
/// alloc_or_diag!(arena, value, "family", errors, return Err(...));
/// ```
#[macro_export]
macro_rules! alloc_or_diag {
    ($arena:expr, $value:expr, $family:literal, $errors:expr) => {
        $crate::alloc_or_diag!($arena, $value, $family, $errors, return)
    };
    ($arena:expr, $value:expr, $family:literal, $errors:expr, $on_overflow:expr) => {{
        match ($arena).alloc($value) {
            ::std::result::Result::Ok(id) => id,
            ::std::result::Result::Err(overflow) => {
                $errors.push(arena_overflow($family, overflow));
                $on_overflow
            }
        }
    }};
}

/// Compact typed arena with deterministic, index-stable ids owned by one typed-core module.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Arena<Id, T> {
    entries: Vec<T>,
    _marker: PhantomData<fn() -> Id>,
}

impl<Id, T> Default for Arena<Id, T> {
    fn default() -> Self {
        Self {
            entries: Vec::new(),
            _marker: PhantomData,
        }
    }
}

impl<Id: ArenaId, T> Arena<Id, T> {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn alloc(&mut self, value: T) -> Result<Id, ArenaOverflow> {
        let index = self.entries.len();
        if index > u32::MAX as usize {
            return Err(ArenaOverflow {
                attempted_len: index,
            });
        }

        let id = Id::from_raw(index as u32);
        self.entries.push(value);
        Ok(id)
    }

    pub fn contains(&self, id: Id) -> bool {
        id.index() < self.entries.len()
    }

    pub fn get(&self, id: Id) -> Option<&T> {
        self.entries.get(id.index())
    }

    pub fn get_mut(&mut self, id: Id) -> Option<&mut T> {
        self.entries.get_mut(id.index())
    }

    pub fn len(&self) -> usize {
        self.entries.len()
    }

    pub fn is_empty(&self) -> bool {
        self.entries.is_empty()
    }

    pub fn iter(&self) -> impl ExactSizeIterator<Item = (Id, &T)> {
        self.entries
            .iter()
            .enumerate()
            .map(|(index, value)| (Id::from_raw(index as u32), value))
    }
}

impl<Id: ArenaId, T> Index<Id> for Arena<Id, T> {
    type Output = T;

    fn index(&self, id: Id) -> &Self::Output {
        &self.entries[id.index()]
    }
}

#[cfg(test)]
mod tests {
    use super::{Arena, ArenaId};

    #[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
    struct TestId(u32);

    impl std::fmt::Display for TestId {
        fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
            write!(f, "{}", self.0)
        }
    }

    impl ArenaId for TestId {
        fn from_raw(raw: u32) -> Self {
            Self(raw)
        }

        fn as_raw(self) -> u32 {
            self.0
        }
    }

    #[test]
    fn allocates_sequential_ids() {
        let mut arena = Arena::<TestId, &str>::new();
        let first = arena.alloc("a").expect("first allocation should fit");
        let second = arena.alloc("b").expect("second allocation should fit");

        assert_eq!(first.as_raw(), 0);
        assert_eq!(second.as_raw(), 1);
        assert_eq!(arena.get(first), Some(&"a"));
        assert_eq!(arena.get(second), Some(&"b"));
    }
}
