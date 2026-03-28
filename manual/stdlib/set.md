# aivi.core.set

Unordered set for any `Eq` type. `Set A` is backed by a deduplicated list. All operations are O(n). Use for small membership collections; for large sets prefer index-backed structures.

```aivi
use aivi.core.set (
    Set
    isEmpty
    singleton
    member
    insert
    remove
    size
    toList
    fromList
    union
    intersection
    difference
    subsetOf
)
```

---

## Type

### `Set`

```aivi
type Set A = { items: List A }
```

An unordered, deduplicated collection of `A` values. The element type `A` can be any type that supports equality. The empty set is the literal `{ items: [] }`.

---

## Construction

### `singleton : A -> Set A`

```aivi
singleton "apple"  // Set Text with one item
```

### `fromList : List A -> Set A`

Build a set from a list, discarding duplicates (first occurrence wins).

```aivi
fromList ["a", "b", "a", "c"]  // { items: ["a", "b", "c"] }
```

---

## Querying

### `isEmpty : Set A -> Bool`

```aivi
isEmpty { items: [] }   // True
isEmpty (singleton "x") // False
```

### `member : A -> Set A -> Bool`

```aivi
let s = fromList ["a", "b", "c"] in
member "b" s  // True
member "z" s  // False
```

### `size : Set A -> Int`

```aivi
fromList ["x", "y", "z"] |> size  // 3
```

### `toList : Set A -> List A`

Returns the items in insertion order.

```aivi
fromList ["b", "a"] |> toList  // ["b", "a"]
```

---

## Modification

### `insert : A -> Set A -> Set A`

Add a value. If already present, the set is unchanged.

```aivi
singleton "a" |> insert "b" |> insert "a"  // { items: ["a", "b"] }
```

### `remove : A -> Set A -> Set A`

Remove a value. No-op if not present.

```aivi
fromList ["a", "b", "c"] |> remove "b"  // { items: ["a", "c"] }
```

---

## Set algebra

### `union : Set A -> Set A -> Set A`

All items from both sets (items from `b` appended when not already in `a`).

```aivi
union (fromList ["a", "b"]) (fromList ["b", "c"])
// { items: ["a", "b", "c"] }
```

### `intersection : Set A -> Set A -> Set A`

Items that appear in both sets.

```aivi
intersection (fromList ["a", "b", "c"]) (fromList ["b", "c", "d"])
// { items: ["b", "c"] }
```

### `difference : Set A -> Set A -> Set A`

Items in `a` that are not in `b`.

```aivi
difference (fromList ["a", "b", "c"]) (fromList ["b"])
// { items: ["a", "c"] }
```

### `subsetOf : Set A -> Set A -> Bool`

`True` when every item in `a` is also in `b`.

```aivi
subsetOf (fromList ["a", "b"]) (fromList ["a", "b", "c"])  // True
subsetOf (fromList ["a", "d"]) (fromList ["a", "b", "c"])  // False
```

---

## Real-world example

```aivi
use aivi.core.set (Set, fromList, member, union, difference, toList)

type TagFilter = { required: Set Text, excluded: Set Text }

fun matchesTags:Bool filter:TagFilter tags:(List Text) =>
    let tagSet = fromList tags in
    let hasRequired = subsetOf filter.required tagSet in
    let hasExcluded = intersection filter.excluded tagSet in
    hasRequired and isEmpty hasExcluded
```
