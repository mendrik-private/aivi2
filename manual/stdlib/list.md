# aivi.list

The `aivi.list` module provides a comprehensive set of functions for working with lists. Lists in AIVI are ordered, immutable sequences of values. All operations return new lists — no mutation ever occurs.

```aivi
use aivi.list (isEmpty, length, map, filter, find, sortBy, ...)
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

```
isEmpty : (List A) -> Bool
```

```aivi
use aivi.list (isEmpty)

fun describe : Text items : List Int =>
    isEmpty items
     T|> "empty list"
     F|> "has elements"

value result : Text = describe []        -- "empty list"
value result2 : Text = describe [1, 2]   -- "has elements"
```

---

### nonEmpty

Returns `True` if the list has at least one element.

```
nonEmpty : (List A) -> Bool
```

```aivi
use aivi.list (nonEmpty)

fun describeList : Text items : List Int =>
    nonEmpty items
     T|> "has elements"
     F|> "empty"

value result : Text = describeList [42]   -- "has elements"
```

---

### length

Returns the number of elements in a list.

```
length : (List A) -> Int
```

```aivi
use aivi.list (length)

value n : Int = length [10, 20, 30]   -- 3
```

---

### head

Returns the first element wrapped in `Some`, or `None` if the list is empty.

```
head : (List A) -> Option A
```

```aivi
use aivi.list (head)

fun firstOrZero : Int items : List Int =>
    head items
     ||> None      -> 0
     ||> Some n    -> n

value result : Int = firstOrZero [5, 10, 15]   -- 5
value fallback : Int = firstOrZero []           -- 0
```

---

### tail

Returns all elements after the first, wrapped in `Some (List A)`, or `None` if the list is empty.

```
tail : (List A) -> Option (List A)
```

```aivi
use aivi.list (tail)

fun restOrEmpty : List Int items : List Int =>
    tail items
     ||> None           -> []
     ||> Some remaining -> remaining

value result : List Int = restOrEmpty [1, 2, 3]   -- [2, 3]
```

---

### tailOrEmpty

Returns all elements after the first, or `[]` if the list is empty. A convenient alternative to `tail` when `None` handling is not needed.

```
tailOrEmpty : (List A) -> List A
```

```aivi
use aivi.list (tailOrEmpty)

value rest : List Int = tailOrEmpty [1, 2, 3]   -- [2, 3]
value none : List Int = tailOrEmpty []           -- []
```

---

### last

Returns the last element wrapped in `Some`, or `None` if the list is empty.

```
last : (List A) -> Option A
```

```aivi
use aivi.list (last)

fun finalScore : Int scores : List Int =>
    last scores
     ||> None      -> 0
     ||> Some n    -> n

value result : Int = finalScore [80, 90, 95]   -- 95
```

---

## Transformation

Functions that produce a new list from an existing one.

### map

Applies a function to every element, returning a new list of the results.

```
map : (A -> B) -> (List A) -> List B
```

```aivi
use aivi.list (map)

fun double : Int n : Int =>
    n * 2

value result : List Int = [1, 2, 3] |> map double   -- [2, 4, 6]
```

---

### filter

Returns only the elements that satisfy a predicate.

```
filter : (A -> Bool) -> (List A) -> List A
```

```aivi
use aivi.list (filter)

fun isPositive : Bool n : Int =>
    n > 0

value result : List Int = [-1, 2, -3, 4] |> filter isPositive   -- [2, 4]
```

---

### flatten

Collapses a list of lists into a single flat list.

```
flatten : (List (List A)) -> List A
```

```aivi
use aivi.list (flatten)

value nested : List (List Int) = [[1, 2], [3, 4], [5]]

value flat : List Int = flatten nested   -- [1, 2, 3, 4, 5]
```

---

### concat

An alias for `flatten`. Joins a list of lists into one.

```
concat : (List (List A)) -> List A
```

```aivi
use aivi.list (concat)

value lines : List (List Text) = [["hello", "world"], ["foo", "bar"]]

value all : List Text = concat lines   -- ["hello", "world", "foo", "bar"]
```

---

### flatMap

Applies a function returning a list to each element, then flattens the result. Equivalent to `map` followed by `flatten`.

```
flatMap : (A -> List B) -> (List A) -> List B
```

```aivi
use aivi.list (flatMap)

fun twice : List Int n : Int =>
    [n, n]

value result : List Int = [1, 2, 3] |> flatMap twice   -- [1, 1, 2, 2, 3, 3]
```

---

### reverse

Returns the list with its elements in reversed order.

```
reverse : (List A) -> List A
```

```aivi
use aivi.list (reverse)

value original : List Int = [1, 2, 3, 4, 5]

value reversed : List Int = reverse original   -- [5, 4, 3, 2, 1]
```

---

### take

Returns the first `n` elements. If the list is shorter than `n`, the entire list is returned.

```
take : Int -> (List A) -> List A
```

```aivi
use aivi.list (take)

value items : List Int = [10, 20, 30, 40, 50]

value first3 : List Int = take 3 items   -- [10, 20, 30]
```

---

### drop

Skips the first `n` elements and returns the rest.

```
drop : Int -> (List A) -> List A
```

```aivi
use aivi.list (drop)

value items : List Int = [10, 20, 30, 40, 50]

value after2 : List Int = drop 2 items   -- [30, 40, 50]
```

---

### takeWhile

Returns the longest prefix of elements that all satisfy the predicate. Stops at the first element that does not match.

```
takeWhile : (A -> Bool) -> (List A) -> List A
```

```aivi
use aivi.list (takeWhile)

fun isSmall : Bool n : Int =>
    n < 10

value result : List Int = [2, 5, 8, 11, 3] |> takeWhile isSmall   -- [2, 5, 8]
```

---

### dropWhile

Drops elements from the front as long as they satisfy the predicate, then returns the rest.

```
dropWhile : (A -> Bool) -> (List A) -> List A
```

```aivi
use aivi.list (dropWhile)

fun isSmall : Bool n : Int =>
    n < 10

value result : List Int = [2, 5, 8, 11, 3] |> dropWhile isSmall   -- [11, 3]
```

---

### intersperse

Inserts a separator element between every pair of adjacent elements.

```
intersperse : A -> (List A) -> List A
```

```aivi
use aivi.list (intersperse)

value words : List Text = ["one", "two", "three"]

value spaced : List Text = intersperse ", " words
-- ["one", ", ", "two", ", ", "three"]
```

---

## Searching

Functions that locate elements or test properties of a list.

### any

Returns `True` if at least one element satisfies the predicate.

```
any : (A -> Bool) -> (List A) -> Bool
```

```aivi
use aivi.list (any)

fun isNegative : Bool n : Int =>
    n < 0

value result : Bool = any isNegative [1, -2, 3]   -- True
```

---

### all

Returns `True` if every element satisfies the predicate.

```
all : (A -> Bool) -> (List A) -> Bool
```

```aivi
use aivi.list (all)

fun isPositive : Bool n : Int =>
    n > 0

value allPositive : Bool = all isPositive [1, 2, 3]    -- True
value someNeg : Bool = all isPositive [1, -2, 3]       -- False
```

---

### count

Returns the number of elements that satisfy the predicate.

```
count : (A -> Bool) -> (List A) -> Int
```

```aivi
use aivi.list (count)

fun isPositive : Bool n : Int =>
    n > 0

value n : Int = count isPositive [-1, 2, 3, -4, 5]   -- 3
```

---

### find

Returns the first element that satisfies the predicate, or `None`.

```
find : (A -> Bool) -> (List A) -> Option A
```

```aivi
use aivi.list (find)

type User = { id: Int, name: Text }

fun hasId : Bool target : Int user : User =>
    user.id == target

fun findUser : Option User id : Int users : List User =>
    find (hasId id) users
```

---

### findMap

Applies a function to each element in order and returns the first `Some` result, or `None` if all calls return `None`. Useful for combined search-and-transform.

```
findMap : (A -> Option B) -> (List A) -> Option B
```

```aivi
use aivi.list (findMap)

fun asPositive : Option Int n : Int =>
    n > 0
     T|> Some n
     F|> None

value result : Option Int = findMap asPositive [-3, -1, 4, 7]   -- Some 4
```

---

### contains

Returns `True` if any element equals the target, using the provided equality function.

```
contains : (A -> A -> Bool) -> A -> (List A) -> Bool
```

```aivi
use aivi.list (contains)

fun intEq : Bool a : Int b : Int =>
    a == b

value found : Bool = contains intEq 3 [1, 2, 3, 4, 5]   -- True
value missing : Bool = contains intEq 9 [1, 2, 3]        -- False
```

---

### indexOf

Returns the index of the first element satisfying the predicate, or `None`.

```
indexOf : (A -> Bool) -> (List A) -> Option Int
```

```aivi
use aivi.list (indexOf)

fun isThirty : Bool n : Int =>
    n == 30

value idx : Option Int = indexOf isThirty [10, 20, 30, 40]   -- Some 2
```

---

## Aggregation

Functions that reduce a list to a single value.

### sum

Sums all integers in a list. Returns `0` for an empty list.

```
sum : (List Int) -> Int
```

```aivi
use aivi.list (sum)

value total : Int = sum [1, 2, 3, 4, 5]   -- 15
```

---

### product

Multiplies all integers in a list together. Returns `1` for an empty list.

```
product : (List Int) -> Int
```

```aivi
use aivi.list (product)

value result : Int = product [1, 2, 3, 4, 5]   -- 120
```

---

### maximum

Returns the largest element wrapped in `Some`, or `None` for an empty list. Requires a comparison function `cmp` where `cmp a b = True` means `a` is less than `b`.

```
maximum : (A -> A -> Bool) -> (List A) -> Option A
```

```aivi
use aivi.list (maximum)

fun isLess : Bool a : Int b : Int =>
    a < b

value highest : Option Int = maximum isLess [3, 1, 4, 1, 5, 9, 2, 6]   -- Some 9
```

---

### minimum

Returns the smallest element wrapped in `Some`, or `None` for an empty list. Accepts the same kind of comparison function as `maximum`.

```
minimum : (A -> A -> Bool) -> (List A) -> Option A
```

```aivi
use aivi.list (minimum)

fun isLess : Bool a : Int b : Int =>
    a < b

value lowest : Option Int = minimum isLess [3, 1, 4, 1, 5, 9, 2, 6]   -- Some 1
```

---

## Set Operations

Functions that treat lists as ordered collections with identity constraints.

### unique

Removes duplicate elements, keeping only the first occurrence of each. Requires an equality function.

```
unique : (A -> A -> Bool) -> (List A) -> List A
```

```aivi
use aivi.list (unique)

fun intEq : Bool a : Int b : Int =>
    a == b

value deduped : List Int = unique intEq [1, 2, 1, 3, 2, 4]   -- [1, 2, 3, 4]
```

---

### sortBy

Sorts a list using an ordering function. The function `cmp a b` should return `True` when `a` should appear before `b` in the result.

```
sortBy : (A -> A -> Bool) -> (List A) -> List A
```

```aivi
use aivi.list (sortBy)

fun intLt : Bool a : Int b : Int =>
    a < b

value sorted : List Int = sortBy intLt [3, 1, 4, 1, 5, 9]   -- [1, 1, 3, 4, 5, 9]
```

For descending order, reverse the comparison:

```aivi
use aivi.list (sortBy)

fun intGt : Bool a : Int b : Int =>
    a > b

value descending : List Int = sortBy intGt [3, 1, 4, 1, 5, 9]   -- [9, 5, 4, 3, 1, 1]
```

---

### partition

Splits a list into two sub-lists: `matched` (elements satisfying the predicate) and `unmatched` (those that do not). Preserves the original order in both sub-lists.

```
partition : (A -> Bool) -> (List A) -> Partition A
```

`Partition A` is a record `{ matched: List A, unmatched: List A }`.

```aivi
use aivi.list (partition)

fun isPositive : Bool n : Int =>
    n > 0

value groups : Partition Int = partition isPositive [-1, 2, -3, 4, -5, 6]

value positive : List Int = groups.matched     -- [2, 4, 6]
value negative : List Int = groups.unmatched   -- [-1, -3, -5]
```

---

## Zipping

Functions that combine multiple lists element-by-element.

### zip

Pairs up elements from two lists into a list of tuples. The result length equals the shorter of the two inputs.

```
zip : (List A) -> (List B) -> List (A, B)
```

```aivi
use aivi.list (zip)

value names : List Text = ["Alice", "Bob", "Carol"]
value scores : List Int = [95, 87, 92]

value pairs : List (Text, Int) = zip names scores
-- [("Alice", 95), ("Bob", 87), ("Carol", 92)]
```

---

### zipWith

Like `zip`, but instead of producing tuples it applies a combining function to each pair of elements.

```
zipWith : (A -> B -> C) -> (List A) -> (List B) -> List C
```

```aivi
use aivi.list (zipWith)

fun add : Int a : Int b : Int =>
    a + b

value sums : List Int = zipWith add [1, 2, 3] [10, 20, 30]   -- [11, 22, 33]
```

---

### unzip

Separates a list of pairs into two separate lists. The inverse of `zip`.

```
unzip : (List (A, B)) -> UnzipState A B
```

`UnzipState A B` is a record `{ lefts: List A, rights: List B }`.

```aivi
use aivi.list (unzip)

value pairs : List (Text, Int) = [("Alice", 95), ("Bob", 87), ("Carol", 92)]

value result : UnzipState Text Int = unzip pairs

value names : List Text = result.lefts    -- ["Alice", "Bob", "Carol"]
value scores : List Int = result.rights   -- [95, 87, 92]
```
