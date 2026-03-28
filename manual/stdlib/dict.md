# aivi.core.dict

Association map keyed by any `Eq` type. `Dict K V` is an ordered association map backed by a list of entries. All operations are `O(n)`. For small to medium-sized dicts this is practical and requires no additional runtime support.

The empty dict is written as the record literal `{ entries: [] }`.

```aivi
use aivi.core.dict (Dict, singleton, insert, insertWith, get, getWithDefault,
                    member, remove, size, keys, values, toList, fromList,
                    mapValues, filterValues, mergeWith, union)
```

---

## Dict

```
type Dict K V = { entries: List (DictEntry K V) }
type DictEntry K V = { key: K, value: V }
```

A `Dict K V` is a record with a single field `entries` holding an association list. The key type `K` can be any type that supports equality. You can construct an empty dict with the record literal directly:

```aivi
use aivi.core.dict (Dict)

value emptyScores:(Dict Text Int) = { entries: [] }
```

---

## singleton

Creates a dict with exactly one entry.

```
singleton : K -> V -> Dict K V
```

```aivi
use aivi.core.dict (singleton)

value greeting:(Dict Text Text) = singleton "hello" "world"
```

---

## insert

Inserts or replaces a key. If the key already exists, the old value is discarded.

```
insert : K -> V -> Dict K V -> Dict K V
```

```aivi
use aivi.core.dict (Dict, insert)

value scores:(Dict Text Int) =
    { entries: [] }
     |> insert "alice" 100
     |> insert "bob" 85
```

---

## insertWith

Inserts a key, combining the new value with the existing one using `merge` if the key is already present.

```
insertWith : (V -> V -> V) -> K -> V -> Dict K V -> Dict K V
```

```aivi
use aivi.core.dict (Dict, insertWith)

fun addScore:(Dict Text Int) key:Text n:Int d:(Dict Text Int) =>
    insertWith (fun total:Int old:Int new:Int => old + new) key n d
```

---

## get

Looks up a key. Returns `None` when the key is absent.

```
get : K -> Dict K V -> Option V
```

```aivi
use aivi.core.dict (Dict, insert, get)

value d:(Dict Text Int) = insert "x" 42 { entries: [] }
value found:(Option Int) = get "x" d
```

---

## getWithDefault

Looks up a key, returning a fallback value when the key is absent.

```
getWithDefault : V -> K -> Dict K V -> V
```

```aivi
use aivi.core.dict (Dict, insert, getWithDefault)

value d:(Dict Text Int) = insert "level" 5 { entries: [] }
value level:Int = getWithDefault 1 "level" d
```

---

## member

Returns `True` if the key exists in the dict.

```
member : K -> Dict K V -> Bool
```

```aivi
use aivi.core.dict (Dict, insert, member)

value d:(Dict Text Int) = insert "exists" 1 { entries: [] }
value hasIt:Bool = member "exists" d
```

---

## remove

Removes a key. Has no effect if the key is absent.

```
remove : K -> Dict K V -> Dict K V
```

```aivi
use aivi.core.dict (Dict, insert, remove)

value d:(Dict Text Int) = insert "temp" 0 { entries: [] }
value cleaned:(Dict Text Int) = remove "temp" d
```

---

## size

Returns the number of entries.

```
size : Dict K V -> Int
```

```aivi
use aivi.core.dict (Dict, insert, size)

value d:(Dict Text Int) =
    { entries: [] }
     |> insert "a" 1
     |> insert "b" 2
value count:Int = size d
```

---

## keys / values

Return the keys or values as a list, in insertion order.

```
keys   : Dict K V -> List K
values : Dict K V -> List V
```

```aivi
use aivi.core.dict (Dict, insert, keys, values)

value d:(Dict Text Int) = insert "score" 99 { entries: [] }
value ks:(List Text) = keys d
value vs:(List Int)  = values d
```

---

## toList / fromList

Convert between a `Dict K V` and a list of `(K, V)` pairs.

```
toList   : Dict K V -> List (K, V)
fromList : List (K, V) -> Dict K V
```

```aivi
use aivi.core.dict (Dict, fromList, toList)

value pairs:(List (Text, Int)) = [("a", 1), ("b", 2)]
value d:(Dict Text Int) = fromList pairs
value back:(List (Text, Int)) = toList d
```

---

## mapValues

Applies a function to every value, preserving keys.

```
mapValues : (V1 -> V2) -> Dict K V1 -> Dict K V2
```

```aivi
use aivi.core.dict (Dict, insert, mapValues)

value d:(Dict Text Int) = insert "pts" 5 { entries: [] }
value doubled:(Dict Text Int) = mapValues (fun n:Int x:Int => x * 2) d
```

---

## filterValues

Keeps only entries whose value satisfies a predicate.

```
filterValues : (V -> Bool) -> Dict K V -> Dict K V
```

```aivi
use aivi.core.dict (Dict, insert, filterValues)

value d:(Dict Text Int) =
    { entries: [] }
     |> insert "low" 3
     |> insert "high" 99
value highOnly:(Dict Text Int) = filterValues (fun b:Bool n:Int => n > 10) d
```

---

## mergeWith

Merges two dicts. When both contain the same key, `combine` is called with the left and right values to produce the merged value.

```
mergeWith : (V -> V -> V) -> Dict K V -> Dict K V -> Dict K V
```

```aivi
use aivi.core.dict (Dict, insert, mergeWith)

value left:(Dict Text Int) = insert "a" 1 { entries: [] }
value right:(Dict Text Int) = insert "a" 10 { entries: [] }
value merged:(Dict Text Int) = mergeWith (fun sum:Int x:Int y:Int => x + y) left right
```

---

## union

Merges two dicts. When a key exists in both, the **right** dict wins.

```
union : Dict K V -> Dict K V -> Dict K V
```

```aivi
use aivi.core.dict (Dict, insert, union)

value defaults:(Dict Text Int) = insert "timeout" 30 { entries: [] }
value overrides:(Dict Text Int) = insert "timeout" 60 { entries: [] }
value config:(Dict Text Int) = union defaults overrides
```
