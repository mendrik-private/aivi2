/// A generic iterative work-list + assembly-stack for structural tree walkers.
///
/// `F` is the frame (work-item) type pushed onto the traversal stack.
/// `A` is the assembled-result type collected bottom-up from child traversals.
///
/// Both [`crate::eq::EqDeriver`] and [`crate::decode::DecodePlanner`] use this to
/// implement their post-order structural walks over [`crate::eq::TypeStore`].
pub struct StructuralWalker<F, A> {
    frames: Vec<F>,
    assembled: Vec<A>,
}

impl<F, A> StructuralWalker<F, A> {
    /// Create a new walker with a single seed frame on the stack.
    pub fn new(initial: F) -> Self {
        Self {
            frames: vec![initial],
            assembled: Vec::new(),
        }
    }

    /// Pop the next frame off the traversal stack. Returns `None` when done.
    pub fn next_frame(&mut self) -> Option<F> {
        self.frames.pop()
    }

    /// Push a frame onto the traversal stack.
    pub fn push_frame(&mut self, frame: F) {
        self.frames.push(frame);
    }

    /// Push an assembled child result.
    pub fn push_assembled(&mut self, item: A) {
        self.assembled.push(item);
    }

    /// Remove and return the last `count` assembled items in order (oldest first).
    ///
    /// # Panics
    /// Panics if fewer than `count` items are assembled.
    pub fn take_tail(&mut self, count: usize) -> Vec<A> {
        let split_at = self
            .assembled
            .len()
            .checked_sub(count)
            .expect("walker: requested more assembled items than available");
        self.assembled.split_off(split_at)
    }

    /// Remove and return the single most-recently assembled item.
    ///
    /// # Panics
    /// Panics if the assembly stack is empty.
    pub fn pop_one(&mut self) -> A {
        self.assembled.pop().expect("walker: assembly stack was empty")
    }

    /// Assert that exactly one item was assembled and return it.
    ///
    /// # Panics
    /// Panics unless exactly one item remains in the assembly stack.
    pub fn finish_one(self) -> A {
        assert_eq!(
            self.assembled.len(),
            1,
            "walker: expected exactly one assembled item at finish"
        );
        self.assembled.into_iter().next().unwrap()
    }
}
