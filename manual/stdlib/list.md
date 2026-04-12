# aivi.list

The `aivi.list` module provides a comprehensive set of functions for working with lists. Lists in AIVI are ordered, immutable sequences of values. All operations return new lists — no mutation ever occurs.

```aivi
use aivi.list (
    isEmpty
    length
    indexed
    map
    mapWithIndex
    reduceWithIndex
    filterMap
    filter
    find
    sort
)
```

Two built-in functions underpin all list work and are available everywhere without importing:

| Function | Signature | Description |
|---|---|---|
| `append` | `List A -> List A -> List A` | Concatenate two lists |
| `reduce` | `(B -> A -> B) -> B -> List A -> B` | Fold a list from left to right |

---

## Inspection

Functions that query the structure or contents of a list without changing it.

### isEmpty

Returns `True` if the list has no elements.

```aivi
```

```aivi
use aivi.list (isEmpty)

type List Int -> Text
func describe = items => isEmpty items
 T|> "empty list"
 F|> "has elements"

value result : Text = describe []

value result2 : Text =
    describe [
        1,
        2
    ]
```

---

### nonEmpty

Returns `True` if the list has at least one element.

```aivi
```

```aivi
use aivi.list (nonEmpty)

type List Int -> Text
func describeList = items => nonEmpty items
 T|> "has elements"
 F|> "empty"

value result : Text =
    describeList [
        42
    ]
```

---

### length

Returns the number of elements in a list.

```aivi
```

```aivi
use aivi.list (length)

value n : Int =
    length [
        10,
        20,
        30
    ]
```

---

### head

Returns the first element wrapped in `Some`, or `None` if the list is empty.

```aivi
```

```aivi
use aivi.list (head)

type List Int -> Int
func firstOrZero = items => head items
 ||> None   -> 0
 ||> Some n -> n

value result : Int =
    firstOrZero [
        5,
        10,
        15
    ]

value fallback : Int = firstOrZero []
```

---

### at

Returns the element at the given zero-based index wrapped in `Some`, or `None` if the index is negative or out of range.

```aivi
```

```aivi
use aivi.list (at)

value items : List Text = [
    "alpha",
    "beta",
    "gamma"
]

value middle : Option Text = at 1 items
value missing : Option Text = at 9 items
```

---

### tail

Returns all elements after the first, wrapped in `Some (List A)`, or `None` if the list is empty.

```aivi
```

```aivi
use aivi.list (tail)

type List Int -> List Int
func restOrEmpty = items => tail items
 ||> None           -> []
 ||> Some remaining -> remaining

value result : List Int =
    restOrEmpty [
        1,
        2,
        3
    ]
```

---

### tailOrEmpty

Returns all elements after the first, or `[]` if the list is empty. A convenient alternative to `tail` when `None` handling is not needed.

```aivi
```

```aivi
use aivi.list (tailOrEmpty)

value rest : List Int =
    tailOrEmpty [
        1,
        2,
        3
    ]

value none : List Int = tailOrEmpty []
```

---

### last

Returns the last element wrapped in `Some`, or `None` if the list is empty.

```aivi
```

```aivi
use aivi.list (last)

type List Int -> Int
func finalScore = scores => last scores
 ||> None   -> 0
 ||> Some n -> n

value result : Int =
    finalScore [
        80,
        90,
        95
    ]
```

---

### indexed

Pairs every element with its zero-based position.

```aivi
use aivi.list (indexed)

value labels : List (Int, Text) =
    indexed [
        "Ada",
        "Grace",
        "Hedy"
    ]
```

---

## Transformation

Functions that produce a new list from an existing one.

### map

Applies a function to every element, returning a new list of the results.

```aivi
```

```aivi
use aivi.list (map)

type Int -> Int
func double = n =>
    n * 2

value result : List Int = [1, 2, 3]
  |> map double
```

---

### mapWithIndex

Maps over a list while also receiving each element's zero-based index.

```aivi
use aivi.list (mapWithIndex)

type Int -> Int -> Int
func offsetByIndex = index item =>
    index + item

value adjusted : List Int =
    mapWithIndex offsetByIndex [
        10,
        20,
        30
    ]
```

---

### filter

Returns only the elements that satisfy a predicate.

```aivi
```

```aivi
use aivi.list (filter)

type Int -> Bool
func isPositive = n =>
    n > 0

value result : List Int =
    filter isPositive [
        -2,
        0,
        3,
        5
    ]
```

---

### filterMap

Applies an `Option`-producing transform and keeps only the `Some` results.

```aivi
use aivi.list (filterMap)

type Int -> (Option Int)
func doubleIfSmall = n => n < 4
 T|> Some (n * 2)
 F|> None

value result : List Int =
    filterMap doubleIfSmall [
        1,
        4,
        2
    ]
```

---

### flatten

Collapses a list of lists into a single flat list.

```aivi
```

```aivi
use aivi.list (flatten)

value nested : List (List Int) = [
    [1, 2],
    [3, 4],
    [5]
]

value flat : List Int = flatten nested
```


---

### flatMap

Applies a function returning a list to each element, then flattens the result. Equivalent to `map` followed by `flatten`.

```aivi
```

```aivi
use aivi.list (flatMap)

type Int -> List Int
func twice = n =>
    [n, n]

value result : List Int = [1, 2, 3]
  |> flatMap twice
```

---

### reduceWithIndex

Folds a list from left to right while also receiving each element's zero-based index.

```aivi
use aivi.list (reduceWithIndex)

type Int -> Int -> Int -> Int
func addIndexed = total index item =>
    total + index + item

value total : Int =
    reduceWithIndex addIndexed 0 [
        10,
        20,
        30
    ]
```

---

### reverse

Returns the list with its elements in reversed order.

```aivi
```

```aivi
use aivi.list (reverse)

value original : List Int = [
    1,
    2,
    3,
    4,
    5
]

value reversed : List Int = reverse original
```

---

### take

Returns the first `n` elements. If the list is shorter than `n`, the entire list is returned.

```aivi
```

```aivi
use aivi.list (take)

value items : List Int = [
    10,
    20,
    30,
    40,
    50
]

value first3 : List Int = take 3 items
```

---

### drop

Skips the first `n` elements and returns the rest.

```aivi
```

```aivi
use aivi.list (drop)

value items : List Int = [
    10,
    20,
    30,
    40,
    50
]

value after2 : List Int = drop 2 items
```

---

### replaceAt

Returns a new list with the element at the given zero-based index replaced. Negative or out-of-range indices leave the original list unchanged.

```aivi
```

```aivi
use aivi.list (replaceAt)

value items : List Int = [
    10,
    20,
    30
]

value updated : List Int = replaceAt 1 99 items
value unchanged : List Int = replaceAt 9 42 items
```

---

### takeWhile

Returns the longest prefix of elements that all satisfy the predicate. Stops at the first element that does not match.

```aivi
```

```aivi
use aivi.list (takeWhile)

type Int -> Bool
func isSmall = n =>
    n < 10

value result : List Int = [2, 5, 8, 11, 3]
  |> takeWhile isSmall
```

---

### dropWhile

Drops elements from the front as long as they satisfy the predicate, then returns the rest.

```aivi
```

```aivi
use aivi.list (dropWhile)

type Int -> Bool
func isSmall = n =>
    n < 10

value result : List Int = [2, 5, 8, 11, 3]
  |> dropWhile isSmall
```

---

### intersperse

Inserts a separator element between every pair of adjacent elements.

```aivi
```

```aivi
use aivi.list (intersperse)

value words : List Text = [
    "one",
    "two",
    "three"
]

value spaced : List Text = intersperse ", " words
```

---

## Searching

Functions that locate elements or test properties of a list.

### any

Returns `True` if at least one element satisfies the predicate.

```aivi
```

```aivi
use aivi.list (any)

type Int -> Bool
func isNegative = n =>
    n < 0

value result : Bool =
    any isNegative [
        1,
        2,
        -3
    ]
```

---

### all

Returns `True` if every element satisfies the predicate.

```aivi
```

```aivi
use aivi.list (all)

type Int -> Bool
func isPositive = n =>
    n > 0

value allPositive : Bool =
    all isPositive [
        1,
        2,
        3
    ]

value someNeg : Bool =
    all isPositive [
        1,
        -2,
        3
    ]
```

---

### count

Returns the number of elements that satisfy the predicate.

```aivi
```

```aivi
use aivi.list (count)

type Int -> Bool
func isPositive = n =>
    n > 0

value n : Int =
    count isPositive [
        -2,
        0,
        3,
        5
    ]
```

---

### find

Returns the first element that satisfies the predicate, or `None`.

```aivi
```

```aivi
use aivi.list (find)

type User = {
    id: Int,
    name: Text
}

type Int -> User -> Bool
func hasId = target user =>
    user.id == target

type Int -> List User -> Option User
func findUser = id users =>
    find (hasId id) users
```

---

### findMap

Applies a function to each element in order and returns the first `Some` result, or `None` if all calls return `None`. Useful for combined search-and-transform.

```aivi
```

```aivi
use aivi.list (findMap)

type Int -> Option Int
func asPositive = n => n > 0
 T|> Some n
 F|> None

value result : Option Int =
    findMap asPositive [
        -2,
        -1,
        4
    ]
```

---

### contains

Returns `True` if any element matches the predicate. The common membership shape is `(. == needle)`.

```aivi
```

```aivi
use aivi.list (contains)

value found : Bool =
    contains (. == 3) [
        1,
        2,
        3,
        4,
        5
    ]

value foundPipe : Bool = [1, 2, 3, 4, 5]
  |> contains (. == 3)

value missing : Bool =
    contains (. == 9) [
        1,
        2,
        3
    ]
```

---

### indexOf

Returns the index of the first element satisfying the predicate, or `None`.

```aivi
```

```aivi
use aivi.list (indexOf)

type Int -> Bool
func isThirty = n =>
    n == 30

value idx : Option Int =
    indexOf isThirty [
        10,
        20,
        30,
        40
    ]
```

---

## Aggregation

Functions that reduce a list to a single value.

### sum

Sums all integers in a list. Returns `0` for an empty list.

```aivi
```

```aivi
use aivi.list (sum)

value total : Int =
    sum [
        1,
        2,
        3,
        4,
        5
    ]
```

---

### product

Multiplies all integers in a list together. Returns `1` for an empty list.

```aivi
```

```aivi
use aivi.list (product)

value result : Int =
    product [
        1,
        2,
        3,
        4,
        5
    ]
```

---

### maximum

Returns the largest element wrapped in `Some`, or `None` for an empty list. Uses the ambient `Ord` instance.

```aivi
```

```aivi
use aivi.list (maximum)

value highest : Option Int =
    maximum [
        3,
        1,
        4,
        1,
        5,
        9,
        2,
        6
    ]
```

For a custom comparator, use `maximumBy`.

---

### minimum

Returns the smallest element wrapped in `Some`, or `None` for an empty list. Uses the ambient `Ord` instance.

```aivi
```

```aivi
use aivi.list (minimum)

value lowest : Option Int =
    minimum [
        3,
        1,
        4,
        1,
        5,
        9,
        2,
        6
    ]
```

For a custom comparator, use `minimumBy`.

---

## Set Operations

Functions that treat lists as ordered collections with identity constraints.

### unique

Removes duplicate elements, keeping only the first occurrence of each. Uses the ambient `Eq` instance.

```aivi
```

```aivi
use aivi.list (unique)

value deduped : List Int =
    unique [
        1,
        2,
        1,
        3,
        2,
        4
    ]
```

For a custom equality relation, use `uniqueBy`.

---

### sort

Sorts a list using the ambient `Ord` instance.

```aivi
use aivi.list (sort)

value sorted : List Int =
    sort [
        3,
        1,
        4,
        1,
        5,
        9
    ]
```

For a custom comparator, use `sortBy`:

```aivi
use aivi.list (sortBy)

type Int -> Int -> Bool
func intGt = a b =>
    a > b

value descending : List Int =
    sortBy intGt [
        3,
        1,
        4,
        1,
        5,
        9
    ]
```

---

### partition

Splits a list into two sub-lists: `matched` (elements satisfying the predicate) and `unmatched` (those that do not). Preserves the original order in both sub-lists.

```aivi
```

`Partition A` is a record `{ matched: List A, unmatched: List A }`.

```aivi
use aivi.list (
    Partition
    partition
)

type Int -> Bool
func isPositive = n =>
    n > 0

value groups : (Partition Int) =
    partition isPositive [
        -2,
        0,
        3,
        5
    ]

value positive : (List Int) = groups
 ||> { matched, unmatched } -> matched

value negative : (List Int) = groups
 ||> { matched, unmatched } -> unmatched
```

---

## Zipping

Functions that combine multiple lists element-by-element.

### zip

Pairs up elements from two lists into a list of tuples. The result length equals the shorter of the two inputs.

```aivi
```

```aivi
use aivi.list (zip)

value names : List Text = [
    "Alice",
    "Bob",
    "Carol"
]

value scores : List Int = [
    95,
    87,
    92
]

value pairs : List (Text, Int) = zip names scores
```

---

### zipWith

Like `zip`, but instead of producing tuples it applies a combining function to each pair of elements.

```aivi
```

```aivi
use aivi.list (zipWith)

type Int -> Int -> Int
func add = a b =>
    a + b

value sums : List Int =
    zipWith add [1, 2, 3] [
        10,
        20,
        30
    ]
```

---

### unzip

Separates a list of pairs into two separate lists. The inverse of `zip`.

```aivi
```

`UnzipState A B` is a record `{ lefts: List A, rights: List B }`.

```aivi
use aivi.list (
    UnzipState
    unzip
)

type UnzipState Text Int -> (List Text)
func takeLefts = state => state
 ||> { lefts, rights } -> lefts

type UnzipState Text Int -> (List Int)
func takeRights = state => state
 ||> { lefts, rights } -> rights

value pairs : List (Text, Int) = [
    ("Alice", 95),
    ("Bob", 87),
    ("Carol", 92)
]

value result : (UnzipState Text Int) = unzip pairs
value names : (List Text) = takeLefts result
value scores : (List Int) = takeRights result
```
