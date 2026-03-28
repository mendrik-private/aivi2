# aivi.core.set

An ordered set of unique `Text` values backed by a deduplicated list. All operations are O(n). Use for small membership collections; for large sets prefer index-backed structures.

```aivi
use aivi.core.set (
    TextSet
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

### `TextSet`

```aivi
type TextSet = { items: List Text }
```

An ordered, deduplicated sequence of `Text` values. The empty set is the literal `{ items: [] }`.

---

## Construction

### `singleton : Text -> TextSet`

```aivi
singleton "apple"  // TextSet with one item
```

### `fromList : List Text -> TextSet`

Build a set from a list, discarding duplicates (first occurrence wins).

```aivi
fromList ["a", "b", "a", "c"]  // { items: ["a", "b", "c"] }
```

---

## Querying

### `isEmpty : TextSet -> Bool`

```aivi
isEmpty { items: [] }   // True
isEmpty (singleton "x") // False
```

### `member : Text -> TextSet -> Bool`

```aivi
let s = fromList ["a", "b", "c"] in
member "b" s  // True
member "z" s  // False
```

### `size : TextSet -> Int`

```aivi
fromList ["x", "y", "z"] |> size  // 3
```

### `toList : TextSet -> List Text`

Returns the items in insertion order.

```aivi
fromList ["b", "a"] |> toList  // ["b", "a"]
```

---

## Modification

### `insert : Text -> TextSet -> TextSet`

Add a value. If already present, the set is unchanged.

```aivi
singleton "a" |> insert "b" |> insert "a"  // { items: ["a", "b"] }
```

### `remove : Text -> TextSet -> TextSet`

Remove a value. No-op if not present.

```aivi
fromList ["a", "b", "c"] |> remove "b"  // { items: ["a", "c"] }
```

---

## Set algebra

### `union : TextSet -> TextSet -> TextSet`

All items from both sets (items from `b` appended when not already in `a`).

```aivi
union (fromList ["a", "b"]) (fromList ["b", "c"])
// { items: ["a", "b", "c"] }
```

### `intersection : TextSet -> TextSet -> TextSet`

Items that appear in both sets.

```aivi
intersection (fromList ["a", "b", "c"]) (fromList ["b", "c", "d"])
// { items: ["b", "c"] }
```

### `difference : TextSet -> TextSet -> TextSet`

Items in `a` that are not in `b`.

```aivi
difference (fromList ["a", "b", "c"]) (fromList ["b"])
// { items: ["a", "c"] }
```

### `subsetOf : TextSet -> TextSet -> Bool`

`True` when every item in `a` is also in `b`.

```aivi
subsetOf (fromList ["a", "b"]) (fromList ["a", "b", "c"])  // True
subsetOf (fromList ["a", "d"]) (fromList ["a", "b", "c"])  // False
```

---

## Real-world example

```aivi
use aivi.core.set (TextSet, fromList, member, union, difference, toList)

type TagFilter = { required: TextSet, excluded: TextSet }

fun matchesTags:Bool filter:TagFilter tags:(List Text) =>
    let tagSet = fromList tags in
    let hasRequired = subsetOf filter.required tagSet in
    let hasExcluded = intersection filter.excluded tagSet in
    hasRequired and isEmpty hasExcluded
```
