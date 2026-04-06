# aivi.core.dict

Association map keyed by any `Eq` type. `Dict K V` is an ordered association map backed by a list of entries. All operations are `O(n)`. For small to medium-sized dicts this is practical and requires no additional runtime support.

The empty dict is written as the record literal `{ entries: [] }`.

```aivi
use aivi.core.dict (
    Dict
    singleton
    insert
    insertWith
    get
    getWithDefault
    member
    remove
    size
    keys
    values
    toList
    fromList
    mapValues
    filterValues
    mergeWith
    union
)
```

---

## Dict

```aivi
type Dict K V = {
    entries: List (DictEntry K V)
}

type DictEntry K V = {
    key: K,
    value: V
}
```

A `Dict K V` is a record with a single field `entries` holding an association list. The key type `K` can be any type that supports equality. You can construct an empty dict with the record literal directly:

```aivi
use aivi.core.dict (Dict)

value emptyScores : (Dict Text Int) = {
    entries: []
}
```

---

## singleton

Creates a dict with exactly one entry.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.dict (
    Dict
    singleton
)

value greeting : (Dict Text Text) = singleton "hello" "world"
```

---

## insert

Inserts or replaces a key. If the key already exists, the old value is discarded.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.dict (
    Dict
    insert
)

value scores : (Dict Text Int) = { entries: [] }
  |> insert "alice" 100
  |> insert "bob" 85
```

---

## insertWith

Inserts a key, combining the new value with the existing one using `merge` if the key is already present.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.dict (
    Dict
    insertWith
)

type Int -> Int -> Int
func addScores = total n =>
    total + n

type Text -> Int -> (Dict Text Int) -> (Dict Text Int)
func addScore = key n d =>
    insertWith addScores key n d
```

---

## get

Looks up a key. Returns `None` when the key is absent.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.dict (
    Dict
    insert
    get
)

value d : (Dict Text Int) =
    insert "x" 42 {
        entries: []
    }

value found : (Option Int) = get "x" d
```

---

## getWithDefault

Looks up a key, returning a fallback value when the key is absent.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.dict (
    Dict
    insert
    getWithDefault
)

value d : (Dict Text Int) =
    insert "level" 5 {
        entries: []
    }

value level : Int = getWithDefault 1 "level" d
```

---

## member

Returns `True` if the key exists in the dict.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.dict (
    Dict
    insert
    member
)

value d : (Dict Text Int) =
    insert "exists" 1 {
        entries: []
    }

value hasIt : Bool = member "exists" d
```

---

## remove

Removes a key. Has no effect if the key is absent.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.dict (
    Dict
    insert
    remove
)

value d : (Dict Text Int) =
    insert "temp" 0 {
        entries: []
    }

value cleaned : (Dict Text Int) = remove "temp" d
```

---

## size

Returns the number of entries.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.dict (
    Dict
    insert
    size
)

value d : (Dict Text Int) = { entries: [] }
  |> insert "a" 1
  |> insert "b" 2

value count : Int = size d
```

---

## keys / values

Return the keys or values as a list, in insertion order.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.dict (
    Dict
    insert
    keys
    values
)

value d : (Dict Text Int) =
    insert "score" 99 {
        entries: []
    }

value ks : (List Text) = keys d
value vs : (List Int) = values d
```

---

## toList / fromList

Convert between a `Dict K V` and a list of `(K, V)` pairs.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.dict (
    Dict
    fromList
    toList
)

value pairs : (List (Text, Int)) = [
    ("a", 1),
    ("b", 2)
]

value d : (Dict Text Int) = fromList pairs
value back : (List (Text, Int)) = toList d
```

---

## mapValues

Applies a function to every value, preserving keys.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.dict (
    Dict
    insert
    mapValues
)

type Int -> Int
func double = n =>
    n * 2

value d : (Dict Text Int) =
    insert "pts" 5 {
        entries: []
    }

value doubled : (Dict Text Int) = mapValues double d
```

---

## filterValues

Keeps only entries whose value satisfies a predicate.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.dict (
    Dict
    insert
    filterValues
)

type Int -> Bool
func isHigh = n =>
    n > 50

value d : (Dict Text Int) = { entries: [] }
  |> insert "low" 3
  |> insert "high" 99

value highOnly : (Dict Text Int) = filterValues isHigh d
```

---

## mergeWith

Merges two dicts. When both contain the same key, `combine` is called with the left and right values to produce the merged value.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.dict (
    Dict
    insert
    mergeWith
)

type Int -> Int -> Int
func addScores = left right =>
    left + right

value left : (Dict Text Int) =
    insert "a" 1 {
        entries: []
    }

value right : (Dict Text Int) =
    insert "a" 10 {
        entries: []
    }

value merged : (Dict Text Int) = mergeWith addScores left right
```

---

## union

Merges two dicts. When a key exists in both, the **right** dict wins.

```aivi
// <unparseable item>
```

```aivi
use aivi.core.dict (
    Dict
    insert
    union
)

value defaults : (Dict Text Int) =
    insert "timeout" 30 {
        entries: []
    }

value overrides : (Dict Text Int) =
    insert "timeout" 60 {
        entries: []
    }

value config : (Dict Text Int) = union defaults overrides
```
