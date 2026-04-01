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
use aivi.core.set (
    Set
    singleton
)

value tags : (Set Text) = singleton "urgent"
```

### `fromList : Eq A -> List A -> Set A`

Build a set from a list, discarding duplicates (first occurrence wins).

```aivi
use aivi.core.set (
    Set
    fromList
)

value tags : (Set Text) =
    fromList [
        "urgent",
        "work",
        "urgent"
    ]
```

---

## Querying

### `isEmpty : Set A -> Bool`

```aivi
use aivi.core.set (
    fromList
    isEmpty
)

value noTags : Bool = isEmpty (fromList [])
```

### `member : Eq A -> A -> Set A -> Bool`

```aivi
use aivi.core.set (
    fromList
    member
)

value hasWork : Bool =
    member "work" (
        fromList [
            "home",
            "work"
        ]
    )
```

### `size : Set A -> Int`

```aivi
use aivi.core.set (
    fromList
    size
)

value tagCount : Int =
    size (
        fromList [
            "a",
            "b",
            "a"
        ]
    )
```

### `toList : Set A -> List A`

Returns the items in insertion order.

```aivi
use aivi.core.set (
    fromList
    toList
)

value items : (List Text) =
    toList (
        fromList [
            "a",
            "b",
            "a"
        ]
    )
```

---

## Modification

### `insert : Eq A -> A -> Set A -> Set A`

Add a value. If already present, the set is unchanged.

```aivi
use aivi.core.set (
    fromList
    insert
)

value tags : (Set Text) =
    insert "work" (
        fromList [
            "home"
        ]
    )
```

### `remove : Eq A -> A -> Set A -> Set A`

Remove a value. No-op if not present.

```aivi
use aivi.core.set (
    fromList
    remove
)

value tags : (Set Text) =
    remove "home" (
        fromList [
            "home",
            "work"
        ]
    )
```

---

## Set algebra

### `union : Eq A -> Set A -> Set A -> Set A`

All items from both sets (items from `b` appended when not already in `a`).

```aivi
use aivi.core.set (
    fromList
    union
)

value merged : (Set Text) =
    union (fromList ["a"]) (
        fromList [
            "b",
            "a"
        ]
    )
```

### `intersection : Eq A -> Set A -> Set A -> Set A`

Items that appear in both sets.

```aivi
use aivi.core.set (
    fromList
    intersection
)

value shared : (Set Text) =
    intersection (fromList ["a", "b"]) (
        fromList [
            "b",
            "c"
        ]
    )
```

### `difference : Eq A -> Set A -> Set A -> Set A`

Items in `a` that are not in `b`.

```aivi
use aivi.core.set (
    fromList
    difference
)

value remaining : (Set Text) =
    difference (fromList ["a", "b"]) (
        fromList [
            "b"
        ]
    )
```

### `subsetOf : Eq A -> Set A -> Set A -> Bool`

`True` when every item in `a` is also in `b`.

```aivi
use aivi.core.set (
    fromList
    subsetOf
)

value isSubset : Bool =
    subsetOf (fromList ["a"]) (
        fromList [
            "a",
            "b"
        ]
    )
```

---

## Real-world example

```aivi
use aivi.core.set (
    Set
    fromList
    isEmpty
    difference
    intersection
)

type TagFilter = {
    required: Set Text,
    excluded: Set Text
}

type TagFilter -> (Set Text) -> Bool
func matchesTagSet = filter tagSet => filter
 ||> { required, excluded } -> isEmpty (difference required tagSet) and isEmpty (intersection excluded tagSet)

type TagFilter -> (List Text) -> Bool
func matchesTags = filter tags =>
    matchesTagSet filter (fromList tags)
```
