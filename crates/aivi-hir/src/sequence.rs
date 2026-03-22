use std::iter;

/// Construction error for fixed-minimum sequence helpers.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum SequenceError {
    Empty,
    TooShort { minimum: usize, found: usize },
}

/// Sequence wrapper that guarantees at least one element.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct NonEmpty<T> {
    first: T,
    rest: Vec<T>,
}

impl<T> NonEmpty<T> {
    pub fn new(first: T, rest: Vec<T>) -> Self {
        Self { first, rest }
    }

    pub fn from_vec(mut values: Vec<T>) -> Result<Self, SequenceError> {
        if values.is_empty() {
            return Err(SequenceError::Empty);
        }

        let rest = values.split_off(1);
        let first = values
            .pop()
            .expect("split_off(1) leaves exactly one element behind");
        Ok(Self { first, rest })
    }

    pub fn first(&self) -> &T {
        &self.first
    }

    pub fn last(&self) -> &T {
        self.rest.last().unwrap_or(&self.first)
    }

    pub fn len(&self) -> usize {
        1 + self.rest.len()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        iter::once(&self.first).chain(self.rest.iter())
    }

    pub fn into_vec(self) -> Vec<T> {
        let mut values = Vec::with_capacity(self.len());
        values.push(self.first);
        values.extend(self.rest);
        values
    }
}

/// Sequence wrapper that guarantees at least two elements.
#[derive(Clone, Debug, PartialEq, Eq, Hash)]
pub struct AtLeastTwo<T> {
    first: T,
    second: T,
    rest: Vec<T>,
}

impl<T> AtLeastTwo<T> {
    pub fn new(first: T, second: T, rest: Vec<T>) -> Self {
        Self {
            first,
            second,
            rest,
        }
    }

    pub fn from_vec(mut values: Vec<T>) -> Result<Self, SequenceError> {
        let found = values.len();
        if found < 2 {
            return Err(SequenceError::TooShort { minimum: 2, found });
        }

        let rest = values.split_off(2);
        let second = values
            .pop()
            .expect("split_off(2) leaves two elements behind");
        let first = values
            .pop()
            .expect("split_off(2) leaves two elements behind");
        Ok(Self {
            first,
            second,
            rest,
        })
    }

    pub fn first(&self) -> &T {
        &self.first
    }

    pub fn second(&self) -> &T {
        &self.second
    }

    pub fn len(&self) -> usize {
        2 + self.rest.len()
    }

    pub fn iter(&self) -> impl DoubleEndedIterator<Item = &T> {
        iter::once(&self.first)
            .chain(iter::once(&self.second))
            .chain(self.rest.iter())
    }

    pub fn into_vec(self) -> Vec<T> {
        let mut values = Vec::with_capacity(self.len());
        values.push(self.first);
        values.push(self.second);
        values.extend(self.rest);
        values
    }
}

#[cfg(test)]
mod tests {
    use super::{AtLeastTwo, NonEmpty, SequenceError};

    #[test]
    fn non_empty_preserves_order() {
        let values = NonEmpty::from_vec(vec![1, 2, 3]).expect("vector is non-empty");
        assert_eq!(values.len(), 3);
        assert_eq!(values.iter().copied().collect::<Vec<_>>(), vec![1, 2, 3]);
    }

    #[test]
    fn non_empty_rejects_empty_vecs() {
        assert_eq!(
            NonEmpty::<u8>::from_vec(Vec::new()),
            Err(SequenceError::Empty)
        );
    }

    #[test]
    fn at_least_two_preserves_order() {
        let values = AtLeastTwo::from_vec(vec!["a", "b", "c"]).expect("vector has three items");
        assert_eq!(values.len(), 3);
        assert_eq!(
            values.iter().copied().collect::<Vec<_>>(),
            vec!["a", "b", "c"]
        );
    }

    #[test]
    fn at_least_two_rejects_short_vecs() {
        assert_eq!(
            AtLeastTwo::<u8>::from_vec(vec![1]),
            Err(SequenceError::TooShort {
                minimum: 2,
                found: 1,
            })
        );
    }
}
