use std::marker::PhantomData;

use crate::RuntimeValue;

/// Storage contract for committed scheduler values.
///
/// Pending evaluator results stay ordinary Rust-owned values until the scheduler commits them at a
/// tick boundary. Stores own only those committed snapshots, which lets the runtime introduce
/// relocation behind stable handles without widening worker/GTK/source boundary contracts.
pub trait CommittedValueStore<V> {
    type Slot: Default;

    fn get<'a>(&'a self, slot: &'a Self::Slot) -> Option<&'a V>;

    fn replace(&mut self, slot: &mut Self::Slot, value: V);

    fn clear(&mut self, slot: &mut Self::Slot) -> bool;

    fn collect(&mut self, roots: &[&Self::Slot]);
}

/// Inline committed-value storage used by non-GC scheduler instantiations.
pub struct InlineCommittedValueStore<V> {
    marker: PhantomData<fn() -> V>,
}

impl<V> Default for InlineCommittedValueStore<V> {
    fn default() -> Self {
        Self {
            marker: PhantomData,
        }
    }
}

impl<V> CommittedValueStore<V> for InlineCommittedValueStore<V> {
    type Slot = Option<V>;

    fn get<'a>(&'a self, slot: &'a Self::Slot) -> Option<&'a V> {
        slot.as_ref()
    }

    fn replace(&mut self, slot: &mut Self::Slot, value: V) {
        *slot = Some(value);
    }

    fn clear(&mut self, slot: &mut Self::Slot) -> bool {
        slot.take().is_some()
    }

    fn collect(&mut self, _roots: &[&Self::Slot]) {}
}

/// Stable root handle for scheduler-owned runtime values.
///
/// The `(slot, generation)` pair never exposes an object address directly. Collections may freely
/// relocate values between spaces while scheduler slots retain the same live handle.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub struct RuntimeGcHandle {
    slot: u32,
    generation: u32,
}

impl RuntimeGcHandle {
    pub const fn slot(self) -> u32 {
        self.slot
    }

    pub const fn generation(self) -> u32 {
        self.generation
    }
}

/// Copying store for committed `RuntimeValue` snapshots.
///
/// This initial moving slice is intentionally narrow: the store owns only committed scheduler
/// values. Each live scheduler slot holds one stable root handle; collections clone reachable
/// values into a fresh space and rewrite root slots, proving relocation without yet widening
/// evaluator-temporary or codegen stack-map contracts.
///
/// # Thread safety
///
/// `MovingRuntimeValueStore` is deliberately not `Send` or `Sync`. The `PhantomData<*mut ()>`
/// field below opts out of both auto-traits. The store must only be accessed from the single
/// thread that owns it. No internal synchronization is provided.
pub struct MovingRuntimeValueStore {
    from_space: RuntimeGcSpace,
    to_space: RuntimeGcSpace,
    roots: Vec<RuntimeGcRootSlot>,
    free_roots: Vec<u32>,
    root_worklist: Vec<RuntimeGcHandle>,
    collections: u64,
    live_roots: usize,
    /// Opts this type out of `Send` and `Sync`. The GC store must only be accessed from its
    /// owning thread; there is no internal synchronization on any field.
    _not_send_sync: PhantomData<*mut ()>,
}

impl Default for MovingRuntimeValueStore {
    fn default() -> Self {
        Self {
            from_space: RuntimeGcSpace::default(),
            to_space: RuntimeGcSpace::default(),
            roots: Vec::new(),
            free_roots: Vec::new(),
            root_worklist: Vec::new(),
            collections: 0,
            live_roots: 0,
            _not_send_sync: PhantomData,
        }
    }
}

impl MovingRuntimeValueStore {
    pub fn collection_count(&self) -> u64 {
        self.collections
    }

    pub fn live_root_count(&self) -> usize {
        self.live_roots
    }

    pub fn allocated_value_count(&self) -> usize {
        self.from_space.values.len()
    }

    fn allocate_root(&mut self, value: RuntimeValue) -> RuntimeGcHandle {
        let object = self.from_space.push(value);
        self.live_roots += 1;
        if let Some(slot_index) = self.free_roots.pop() {
            let slot = &mut self.roots[slot_index as usize];
            debug_assert!(
                slot.object.is_none(),
                "free root slots must not keep live object ids"
            );
            slot.object = Some(object);
            RuntimeGcHandle {
                slot: slot_index,
                generation: slot.generation,
            }
        } else {
            let slot_index = self.roots.len() as u32;
            self.roots.push(RuntimeGcRootSlot {
                generation: 0,
                object: Some(object),
            });
            RuntimeGcHandle {
                slot: slot_index,
                generation: 0,
            }
        }
    }

    fn root_slot(&self, handle: RuntimeGcHandle) -> &RuntimeGcRootSlot {
        let slot = self
            .roots
            .get(handle.slot as usize)
            .expect("moving-GC root handles must reference an allocated slot");
        assert_eq!(
            slot.generation, handle.generation,
            "moving-GC root handles must not outlive their generation"
        );
        slot
    }

    // SAFETY: There is no synchronization on this accessor. `MovingRuntimeValueStore` is neither
    // `Send` nor `Sync` (see the `_not_send_sync: PhantomData<*mut ()>` field), so the borrow
    // checker enforces that only one thread can ever hold a reference to the store at a time.
    // Calling this from multiple threads without external synchronization is undefined behavior
    // and would allow data races on the root-slot vector.
    fn root_slot_mut(&mut self, handle: RuntimeGcHandle) -> &mut RuntimeGcRootSlot {
        let slot = self
            .roots
            .get_mut(handle.slot as usize)
            .expect("moving-GC root handles must reference an allocated slot");
        assert_eq!(
            slot.generation, handle.generation,
            "moving-GC root handles must not outlive their generation"
        );
        slot
    }

    fn resolve_handle(&self, handle: RuntimeGcHandle) -> &RuntimeValue {
        let object = self
            .root_slot(handle)
            .object
            .expect("moving-GC root handles must always point at a live object");
        self.from_space.get(object)
    }

    fn recycle_root(&mut self, handle: RuntimeGcHandle) -> bool {
        let slot = self.root_slot_mut(handle);
        let had_object = slot.object.take().is_some();
        if had_object {
            slot.generation = slot.generation.wrapping_add(1);
            self.live_roots = self
                .live_roots
                .checked_sub(1)
                .expect("clearing a live root must not underflow the root count");
            self.free_roots.push(handle.slot);
        }
        had_object
    }
}

impl CommittedValueStore<RuntimeValue> for MovingRuntimeValueStore {
    type Slot = Option<RuntimeGcHandle>;

    fn get<'a>(&'a self, slot: &'a Self::Slot) -> Option<&'a RuntimeValue> {
        slot.as_ref().map(|handle| self.resolve_handle(*handle))
    }

    fn replace(&mut self, slot: &mut Self::Slot, value: RuntimeValue) {
        if let Some(handle) = slot.as_ref().copied() {
            let object = self
                .root_slot(handle)
                .object
                .expect("live moving-GC roots must point at an allocated object");
            // TODO: write barrier — this direct in-place write bypasses any GC write barrier.
            // A generational or incremental collector requires a write barrier here to record
            // that the old-generation object at `object` has been overwritten with a potentially
            // young-generation value. Without introducing barriers at every write site like this
            // one, neither a generational nor an incremental GC can be added safely.
            self.from_space.values[object.0 as usize] = value;
            return;
        }

        *slot = Some(self.allocate_root(value));
    }

    fn clear(&mut self, slot: &mut Self::Slot) -> bool {
        let Some(handle) = slot.take() else {
            return false;
        };
        self.recycle_root(handle)
    }

    fn collect(&mut self, roots: &[&Self::Slot]) {
        self.root_worklist.clear();
        self.root_worklist
            .extend(roots.iter().filter_map(|slot| slot.as_ref().copied()));
        self.to_space.values.clear();
        self.to_space.values.reserve(self.root_worklist.len());
        let worklist = self.root_worklist.clone();
        for handle in worklist {
            let relocated = {
                let value = self.resolve_handle(handle).clone();
                self.to_space.push(value)
            };
            self.root_slot_mut(handle).object = Some(relocated);
        }

        std::mem::swap(&mut self.from_space, &mut self.to_space);
        self.to_space.values.clear();
        self.collections = self.collections.wrapping_add(1);
    }
}

#[derive(Default)]
struct RuntimeGcSpace {
    values: Vec<RuntimeValue>,
}

impl RuntimeGcSpace {
    fn push(&mut self, value: RuntimeValue) -> RuntimeGcObjectId {
        let index = self.values.len() as u32;
        self.values.push(value);
        RuntimeGcObjectId(index)
    }

    fn get(&self, id: RuntimeGcObjectId) -> &RuntimeValue {
        self.values
            .get(id.0 as usize)
            .expect("moving-GC object ids must reference the active space")
    }
}

#[derive(Clone, Copy)]
struct RuntimeGcObjectId(u32);

struct RuntimeGcRootSlot {
    generation: u32,
    object: Option<RuntimeGcObjectId>,
}

#[cfg(test)]
mod tests {
    use super::{CommittedValueStore, MovingRuntimeValueStore, RuntimeGcHandle, RuntimeValue};

    fn text_ptr(value: &RuntimeValue) -> *const u8 {
        let RuntimeValue::Text(text) = value else {
            panic!("expected text runtime value");
        };
        text.as_ptr()
    }

    #[test]
    fn moving_store_relocates_text_roots_without_changing_handles() {
        let mut store = MovingRuntimeValueStore::default();
        let mut slot = Option::<RuntimeGcHandle>::default();
        store.replace(&mut slot, RuntimeValue::Text("Ada".into()));
        let handle = slot.expect("store should allocate a root handle");
        let before_value = store
            .get(&slot)
            .expect("allocated root should remain readable")
            as *const RuntimeValue;
        let before_text = text_ptr(store.get(&slot).unwrap());

        let roots = [&slot];
        store.collect(&roots);

        assert_eq!(slot, Some(handle));
        assert_eq!(store.collection_count(), 1);
        assert_eq!(store.live_root_count(), 1);
        assert_eq!(store.allocated_value_count(), 1);
        let after = store
            .get(&slot)
            .expect("collected root should stay readable");
        assert_eq!(after, &RuntimeValue::Text("Ada".into()));
        assert_ne!(
            before_value, after as *const RuntimeValue,
            "moving collection must relocate the committed value object"
        );
        assert_ne!(
            before_text,
            text_ptr(after),
            "moving collection must relocate nested text storage too"
        );
    }

    #[test]
    fn moving_store_clears_and_reuses_slots_with_new_generations() {
        let mut store = MovingRuntimeValueStore::default();
        let mut slot = Option::<RuntimeGcHandle>::default();
        store.replace(&mut slot, RuntimeValue::Text("old".into()));
        let first = slot.expect("first allocation should produce a handle");

        assert!(store.clear(&mut slot));
        assert_eq!(slot, None);
        assert_eq!(store.live_root_count(), 0);
        assert_eq!(
            store.allocated_value_count(),
            1,
            "dead objects stay in from-space until the next collection"
        );

        let no_roots: [&Option<RuntimeGcHandle>; 0] = [];
        store.collect(&no_roots);
        assert_eq!(store.allocated_value_count(), 0);

        store.replace(&mut slot, RuntimeValue::Text("new".into()));
        let second = slot.expect("re-allocation should produce a new handle");
        assert_eq!(
            first.slot(),
            second.slot(),
            "cleared root slots should be recycled instead of leaking"
        );
        assert_ne!(
            first.generation(),
            second.generation(),
            "recycled root slots must advance generation to invalidate stale handles"
        );
        assert_eq!(store.live_root_count(), 1);
    }
}
