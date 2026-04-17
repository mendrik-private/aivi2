const AMBIENT_PRELUDE_SOURCE: &str = r#"type Ordering = Less | Equal | Greater

class Setoid A = {
    equals : A -> A -> Bool
}

class Semigroupoid C = {
    compose : C B C -> C A B -> C A C
}

class Semigroup A = {
    append : A -> A -> A
}

class Foldable F = {
    reduce : (B -> A -> B) -> B -> F A -> B
}

class Functor F = {
    map : (A -> B) -> F A -> F B
}

class Contravariant F = {
    contramap : (B -> A) -> F A -> F B
}

class Filterable F = {
    with Functor F
    filterMap : (A -> Option B) -> F A -> F B
}

class Eq A = {
    (==) : A -> A -> Bool
    (!=) : A -> A -> Bool
}

class Default A = {
    default : A
}

class Ord A = {
    with Eq A
    compare : A -> A -> Ordering
}

class Category C = {
    with Semigroupoid C
    id : C A A
}

class Monoid A = {
    with Semigroup A
    empty : A
}

class Traversable T = {
    with Functor T
    with Foldable T
    traverse : Applicative G => (A -> G B) -> T A -> G (T B)
}

class Profunctor P = {
    dimap : (A2 -> A1) -> (B1 -> B2) -> P A1 B1 -> P A2 B2
}

class Bifunctor F = {
    bimap : (A -> C) -> (B -> D) -> F A B -> F C D
}

class Group A = {
    with Monoid A
    invert : A -> A
}

class Alt F = {
    with Functor F
    alt : F A -> F A -> F A
}

class Apply F = {
    with Functor F
    apply : F (A -> B) -> F A -> F B
}

class Extend W = {
    with Functor W
    extend : (W A -> B) -> W A -> W B
}

class Plus F = {
    with Alt F
    zero : F A
}

class Applicative F = {
    with Apply F
    pure : A -> F A
}

class Chain M = {
    with Apply M
    chain : (A -> M B) -> M A -> M B
}

class Comonad W = {
    with Extend W
    extract : W A -> A
}

class Alternative F = {
    with Applicative F
    with Plus F
    guard : Bool -> F Unit
}

class Monad M = {
    with Applicative M
    with Chain M
    join : M (M A) -> M A
}

class ChainRec M = {
    with Monad M
    chainRec : (A -> M (Result A B)) -> A -> M B
}

type __AiviListTailState A = {
    seenFirst: Bool,
    items: List A
}

type A -> (Option A) -> A
func __aivi_option_getOrElse = fallback opt => opt
    ||> Some item -> item
    ||> None      -> fallback

type A -> (Option A)
func __aivi_list_keepSome = item =>
    Some item

type A -> Result E A -> A
func __aivi_result_withDefault = fallback result => result
    ||> Ok item -> item
    ||> Err _   -> fallback

type (Option A) -> A -> (Option A)
func __aivi_list_keepFirst = found item => found
    T|> __aivi_list_keepSome
    F|> Some item

type Int -> A -> Int
func __aivi_list_lengthStep = total item =>
    total + 1

type (List A) -> Int
func __aivi_list_length = items =>
    items
      |> reduce __aivi_list_lengthStep 0

type (List A) -> (Option A)
func __aivi_list_head = items =>
    items
      |> reduce __aivi_list_keepFirst None

type (List A) -> A -> Bool -> (__AiviListTailState A)
func __aivi_list_tailState = items item seenFirst => seenFirst
    T|> { seenFirst: True, items: append items [item] }
    F|> { seenFirst: True, items: [] }

type (__AiviListTailState A) -> A -> (__AiviListTailState A)
func __aivi_list_tailStep = state item => state
    ||> { seenFirst, items } -> __aivi_list_tailState items item seenFirst

type (List A) -> Bool -> (Option (List A))
func __aivi_list_tailItems = items seenFirst => seenFirst
    T|> Some items
    F|> None

type (__AiviListTailState A) -> (Option (List A))
func __aivi_list_tailFromState = state => state
    ||> { seenFirst, items } -> __aivi_list_tailItems items seenFirst

type (List A) -> (Option (List A))
func __aivi_list_tail = items =>
    items
      |> reduce __aivi_list_tailStep { seenFirst: False, items: [] }
      |> __aivi_list_tailFromState

type (List A) -> (List A)
func __aivi_list_tailOrEmpty = items =>
    __aivi_list_tail items
        ||> Some remaining -> remaining
        ||> None           -> []

type (List A) -> Bool
func __aivi_list_nonEmpty = items =>
    __aivi_list_head items
      T|> True
      F|> False

type (A -> Bool) -> Bool -> A -> Bool
func __aivi_list_anyStep = predicate found item => found
    T|> True
    F|> predicate item

type (A -> Bool) -> (List A) -> Bool
func __aivi_list_any = predicate items =>
    items
      |> reduce (__aivi_list_anyStep predicate) False

type Eq A => A -> A -> Bool
func __aivi_binary_eq = left right =>
    left == right

type Eq A => A -> A -> Bool
func __aivi_binary_neq = left right =>
    left != right

type Ord A => A -> A -> Bool
func __aivi_binary_lt = left right =>
    compare left right
     ||> Less    -> True
     ||> Equal   -> False
     ||> Greater -> False

type Ord A => A -> A -> Bool
func __aivi_binary_gt = left right =>
    compare left right
     ||> Less    -> False
     ||> Equal   -> False
     ||> Greater -> True

type Ord A => A -> A -> Bool
func __aivi_binary_lte = left right =>
    compare left right
     ||> Greater -> False
     ||> _       -> True

type Ord A => A -> A -> Bool
func __aivi_binary_gte = left right =>
    compare left right
     ||> Less -> False
     ||> _    -> True

type Ord A => A -> A -> A
func __aivi_order_min = left right =>
    right < left
    T|> right
    F|> left

type Ord A => A -> A -> A
func __aivi_order_max = left right =>
    left < right
    T|> right
    F|> left

type Ord A => A -> (List A) -> A
func __aivi_order_minOf = first rest =>
    __aivi_list_minimumFrom first rest

type Ord A => A -> (List A) -> A
func __aivi_order_maxOf = first rest =>
    __aivi_list_maximumFrom first rest

type Ord A => A -> A -> A
func __aivi_order_clampToMax = high value =>
    high < value
    T|> high
    F|> value

type Ord A => A -> A -> A -> A
func __aivi_order_clamp = low high value =>
    value < low
    T|> low
    F|> __aivi_order_clampToMax high value

type Ord A => A -> A -> A
func min = left right =>
    __aivi_order_min left right

type Ord A => A -> A -> A
func max = left right =>
    __aivi_order_max left right

type Ord A => A -> (List A) -> A
func minOf = first rest =>
    __aivi_order_minOf first rest

type Ord A => A -> (List A) -> A
func maxOf = first rest =>
    __aivi_order_maxOf first rest

type Ord A => A -> A -> A -> A
func clamp = low high value =>
    __aivi_order_clamp low high value

domain NonEmptyList A over List A = {
    type (List A) -> (NonEmptyList A)
    lift items = items

    type NonEmptyList A -> (List A)
    __aivi_nel_carrier nel = nel
}

type A -> (NonEmptyList A)
func __aivi_nel_singleton = item =>
    lift [item]

type A -> (NonEmptyList A) -> (NonEmptyList A)
func __aivi_nel_cons = item nel =>
    lift (append [item] (__aivi_nel_carrier nel))

type NonEmptyList A -> A
func __aivi_nel_head = nel =>
    __aivi_nel_carrier nel
    ||> [h, ...ignored] -> h

type NonEmptyList A -> (List A)
func __aivi_nel_toList = nel =>
    __aivi_nel_carrier nel

type A -> (List A) -> (NonEmptyList A)
func __aivi_nel_fromHeadTail = h t =>
    lift (append [h] t)

type NonEmptyList A -> Int
func __aivi_nel_length = nel =>
    __aivi_list_length (__aivi_nel_carrier nel)

type A -> A -> A
func __aivi_nel_lastStep = prev item =>
    item

type A -> (List A) -> A
func __aivi_nel_lastOf = h t => t
    |> reduce __aivi_nel_lastStep h

type NonEmptyList A -> A
func __aivi_nel_last = nel =>
    __aivi_nel_carrier nel
    ||> [h, ...t] -> __aivi_nel_lastOf h t

type (A -> B) -> (NonEmptyList A) -> (NonEmptyList B)
func __aivi_nel_mapNel = transform nel =>
    lift (__aivi_list_map transform (__aivi_nel_carrier nel))

type (NonEmptyList A) -> (NonEmptyList A) -> (NonEmptyList A)
func __aivi_nel_appendNel = left right =>
    lift (append (__aivi_nel_carrier left) (__aivi_nel_carrier right))

type (List A) -> (Option A) -> (List A)
func __aivi_nel_initAppendPrev = items prev => prev
    ||> Some p -> append items [p]
    ||> None   -> items

type (List A) -> (Option A) -> A -> (List A, Option A)
func __aivi_nel_initAccum = items prev item =>
    (__aivi_nel_initAppendPrev items prev, Some item)

type (List A, Option A) -> A -> (List A, Option A)
func __aivi_nel_initStep = state item => state
    ||> (items, prev) -> __aivi_nel_initAccum items prev item

type (List A, Option A) -> (List A)
func __aivi_nel_initExtract = state => state
    ||> (items, prev) -> items

type NonEmptyList A -> (List A)
func __aivi_nel_init = nel =>
    __aivi_nel_carrier nel
    |> reduce __aivi_nel_initStep ([], None)
    |> __aivi_nel_initExtract

type (Option (NonEmptyList A)) -> A -> (Option (NonEmptyList A))
func __aivi_nel_fromListStep = acc item => acc
    ||> None     -> Some (lift [item])
    ||> Some nel -> Some (lift (append (__aivi_nel_carrier nel) [item]))

type (List A) -> (Option (NonEmptyList A))
func __aivi_nel_fromList = items => items
    |> reduce __aivi_nel_fromListStep None

type (A -> B) -> (Option A) -> (Option B)
func __aivi_option_map = transform opt => opt
    ||> Some item -> Some (transform item)
    ||> None      -> None

type Eq A => A -> (List A) -> Bool
func __aivi_list_contains = target items =>
    __aivi_list_containsEq target items

type (A -> A -> Bool) -> (List A) -> A -> (List A)
func __aivi_list_uniqueByStep = eq acc item =>
    __aivi_list_any (eq item) acc
      T|> acc
      F|> append acc [item]

type (A -> A -> Bool) -> (List A) -> (List A)
func __aivi_list_uniqueBy = eq items => items
    |> reduce (__aivi_list_uniqueByStep eq) []

type Eq A => A -> Bool -> A -> Bool
func __aivi_list_containsEqStep = target found item => found
    T|> True
    F|> __aivi_binary_eq target item

type Eq A => A -> (List A) -> Bool
func __aivi_list_containsEq = target items => items
    |> reduce (__aivi_list_containsEqStep target) False

type Eq A => (List A) -> A -> (List A)
func __aivi_list_uniqueEqStep = acc item =>
    __aivi_list_containsEq item acc
      T|> acc
      F|> append acc [item]

type Eq A => (List A) -> (List A)
func __aivi_list_unique = items =>
    items
      |> reduce __aivi_list_uniqueEqStep []

type Eq A => (List A) -> (List A)
func unique = items =>
    __aivi_list_unique items

type (A -> B) -> (List B) -> A -> (List B)
func __aivi_list_mapStep = transform acc item =>
    append acc [transform item]

type (A -> B) -> (List A) -> (List B)
func __aivi_list_map = transform items => items
    |> reduce (__aivi_list_mapStep transform) []

type (A -> List B) -> (List B) -> A -> (List B)
func __aivi_list_flatMapStep = transform acc item =>
    append acc (transform item)

type (A -> List B) -> (List A) -> (List B)
func __aivi_list_flatMap = transform items => items
    |> reduce (__aivi_list_flatMapStep transform) []

type (A -> Bool) -> (List A) -> A -> (List A)
func __aivi_list_filterAppend = predicate acc item => predicate item
    T|> append acc [item]
    F|> acc

type (A -> Bool) -> (List A) -> (List A)
func __aivi_list_filter = predicate items => items
    |> reduce (__aivi_list_filterAppend predicate) []

type (A -> Bool) -> Int -> A -> Int
func __aivi_list_countStep = predicate acc item => predicate item
    T|> acc + 1
    F|> acc

type (A -> Bool) -> (List A) -> Int
func __aivi_list_count = predicate items => items
    |> reduce (__aivi_list_countStep predicate) 0

type Int -> Int -> Int
func __aivi_list_sumStep = acc item =>
    acc + item

type (List Int) -> Int
func __aivi_list_sum = items => items
    |> reduce __aivi_list_sumStep 0

type (A -> A -> Bool) -> A -> A -> (Option A)
func __aivi_list_maxPick = gt item prev => gt item prev
    T|> Some item
    F|> Some prev

type (A -> A -> Bool) -> (Option A) -> A -> (Option A)
func __aivi_list_maximumStep = gt best item => best
    ||> None     -> Some item
    ||> Some prev -> __aivi_list_maxPick gt item prev

type (A -> A -> Bool) -> (List A) -> (Option A)
func __aivi_list_maximum = gt items => items
    |> reduce (__aivi_list_maximumStep gt) None

type Ord A => A -> A -> (Option A)
func __aivi_list_maximumOrdPick = item prev =>
    __aivi_binary_gt item prev
      T|> Some item
      F|> Some prev

type Ord A => (Option A) -> A -> (Option A)
func __aivi_list_maximumOrdStep = best item => best
    ||> None      -> Some item
    ||> Some prev -> __aivi_list_maximumOrdPick item prev

type Ord A => A -> A -> A
func __aivi_list_maximumFromStep = best item =>
    __aivi_binary_gt item best
      T|> item
      F|> best

type Ord A => A -> (List A) -> A
func __aivi_list_maximumFrom = best items => items
    |> reduce __aivi_list_maximumFromStep best

type Ord A => A -> A -> (Option A)
func __aivi_list_minimumOrdPick = item prev =>
    __aivi_binary_lt item prev
      T|> Some item
      F|> Some prev

type Ord A => (Option A) -> A -> (Option A)
func __aivi_list_minimumOrdStep = best item => best
    ||> None      -> Some item
    ||> Some prev -> __aivi_list_minimumOrdPick item prev

type Ord A => A -> A -> A
func __aivi_list_minimumFromStep = best item =>
    __aivi_binary_lt item best
      T|> item
      F|> best

type Ord A => A -> (List A) -> A
func __aivi_list_minimumFrom = best items => items
    |> reduce __aivi_list_minimumFromStep best

type Ord A => (List A) -> (Option A)
func maximum = items =>
    items
      |> reduce __aivi_list_maximumOrdStep None

type Ord A => (List A) -> (Option A)
func minimum = items =>
    items
      |> reduce __aivi_list_minimumOrdStep None

type Int -> (List Int) -> (List Int)
func __aivi_list_rangeDesc = current acc => current < 0
    T|> acc
    F|> __aivi_list_rangeDesc (current - 1) (append [current] acc)

type Int -> (List Int)
func __aivi_list_range = n => n <= 0
    T|> []
    F|> __aivi_list_rangeDesc (n - 1) []

type Text -> Text -> Text -> (Bool, Text)
func __aivi_text_joinFirst = sep result item =>
    (False, item)

type Text -> Text -> Text -> (Bool, Text)
func __aivi_text_joinNext = sep result item =>
    (False, append (append result sep) item)

type Bool -> Text -> Text -> Text -> (Bool, Text)
func __aivi_text_joinPick = isFirst sep result item => isFirst
    T|> __aivi_text_joinFirst sep result item
    F|> __aivi_text_joinNext sep result item

type Text -> (Bool, Text) -> Text -> (Bool, Text)
func __aivi_text_joinStep = sep state item => state
    ||> (isFirst, result) -> __aivi_text_joinPick isFirst sep result item

type (Bool, Text) -> Text
func __aivi_text_joinExtract = state => state
    ||> (isFirst, result) -> result

type Text -> (List Text) -> Text
func __aivi_text_join = sep items => items
    |> reduce (__aivi_text_joinStep sep) (True, "")
    |> __aivi_text_joinExtract

type Matrix A =
  | MkMatrix Int Int (List (List A))

type MatrixError =
  | NegativeWidth Int
  | NegativeHeight Int
  | RaggedRows Int Int Int

type (Matrix A) -> (List (List A))
func __aivi_matrix_rows = matrix => matrix
    ||> MkMatrix w h data -> data

type (Matrix A) -> Int
func __aivi_matrix_width = matrix => matrix
    ||> MkMatrix w h data -> w

type (Matrix A) -> Int
func __aivi_matrix_height = matrix => matrix
    ||> MkMatrix w h data -> h

type Bool -> Int -> A -> (Int, Option A)
func __aivi_listAt_match = matches idx item => matches
    T|> (idx + 1, Some item)
    F|> (idx + 1, None)

type Int -> Int -> (Option A) -> A -> (Int, Option A)
func __aivi_listAt_check = target idx found item => found
    ||> Some already -> (idx + 1, Some already)
    ||> None -> __aivi_listAt_match (idx == target) idx item

type Int -> (Int, Option A) -> A -> (Int, Option A)
func __aivi_listAt_step = target state item => state
    ||> (idx, found) -> __aivi_listAt_check target idx found item

type (Int, Option A) -> (Option A)
func __aivi_listAt_extract = state => state
    ||> (idx, found) -> found

type Int -> (List A) -> (Option A)
func __aivi_listAt = target items => items
    |> reduce (__aivi_listAt_step target) (0, None)
    |> __aivi_listAt_extract

type (Option (List A)) -> Int -> (Option A)
func __aivi_matrix_atRow = rowOpt x => rowOpt
    ||> Some row -> __aivi_listAt x row
    ||> None     -> None

type (Matrix A) -> Int -> Int -> (Option A)
func __aivi_matrix_at = matrix x y => matrix
    ||> MkMatrix w h data ->
        __aivi_matrix_atRow (__aivi_listAt y data) x

type Bool -> A -> Int -> (List A) -> A -> (Int, List A)
func __aivi_listReplace_pick = matches newVal idx result item => matches
    T|> (idx + 1, append result [newVal])
    F|> (idx + 1, append result [item])

type Int -> A -> Int -> (List A) -> A -> (Int, List A)
func __aivi_listReplace_check = target newVal idx result item =>
    __aivi_listReplace_pick (idx == target) newVal idx result item

type Int -> A -> (Int, List A) -> A -> (Int, List A)
func __aivi_listReplace_step = target newVal state item => state
    ||> (idx, result) -> __aivi_listReplace_check target newVal idx result item

type (Int, List A) -> (List A)
func __aivi_listReplace_extract = state => state
    ||> (idx, result) -> result

type Int -> A -> (List A) -> (List A)
func __aivi_listReplace = target newVal items => items
    |> reduce (__aivi_listReplace_step target newVal) (0, [])
    |> __aivi_listReplace_extract

type Int -> Int -> Int -> Int -> (List (List A)) -> A -> (Option (Matrix A))
func __aivi_matrix_doReplace = x y w h data value =>
    __aivi_listAt y data
        ||> Some row -> Some (MkMatrix w h (__aivi_listReplace y (__aivi_listReplace x value row) data))
        ||> None     -> None

type Bool -> Bool -> Bool -> Bool -> Int -> Int -> Int -> Int -> (List (List A)) -> A -> (Option (Matrix A))
func __aivi_matrix_boundsCheck = xOk yOk xLt yLt x y w h data value => xOk
    T|> __aivi_matrix_boundsCheck2 yOk xLt yLt x y w h data value
    F|> None

type Bool -> Bool -> Bool -> Int -> Int -> Int -> Int -> (List (List A)) -> A -> (Option (Matrix A))
func __aivi_matrix_boundsCheck2 = yOk xLt yLt x y w h data value => yOk
    T|> __aivi_matrix_boundsCheck3 xLt yLt x y w h data value
    F|> None

type Bool -> Bool -> Int -> Int -> Int -> Int -> (List (List A)) -> A -> (Option (Matrix A))
func __aivi_matrix_boundsCheck3 = xLt yLt x y w h data value => xLt
    T|> __aivi_matrix_boundsCheck4 yLt x y w h data value
    F|> None

type Bool -> Int -> Int -> Int -> Int -> (List (List A)) -> A -> (Option (Matrix A))
func __aivi_matrix_boundsCheck4 = yLt x y w h data value => yLt
    T|> __aivi_matrix_doReplace x y w h data value
    F|> None

type (Matrix A) -> Int -> Int -> A -> (Option (Matrix A))
func __aivi_matrix_replaceCoord = matrix x y value => matrix
    ||> MkMatrix w h data -> __aivi_matrix_boundsCheck (x >= 0) (y >= 0) (x < w) (y < h) x y w h data value

type (Matrix A) -> (Int, Int) -> A -> (Option (Matrix A))
func __aivi_matrix_replaceAt = matrix coord value => coord
    ||> (x, y) -> __aivi_matrix_replaceCoord matrix x y value

type (Matrix A) -> ((Int, Int), A) -> (Option (Matrix A))
func __aivi_matrix_replaceManyUpdate = matrix update => update
    ||> (coord, value) -> __aivi_matrix_replaceAt matrix coord value

type (Option (Matrix A)) -> ((Int, Int), A) -> (Option (Matrix A))
func __aivi_matrix_replaceManyStep = current update => current
    ||> None        -> None
    ||> Some matrix -> __aivi_matrix_replaceManyUpdate matrix update

type (Matrix A) -> (List ((Int, Int), A)) -> (Option (Matrix A))
func __aivi_matrix_replaceMany = matrix updates =>
    updates |> reduce __aivi_matrix_replaceManyStep (Some matrix)

type Int -> Int -> (List A) -> (Int, Int, Option MatrixError)
func __aivi_matrix_validateFirstRow = rowIdx expectedWidth row =>
    (1, __aivi_list_length row, None)

type Bool -> Int -> Int -> (List A) -> (Int, Int, Option MatrixError)
func __aivi_matrix_validateLengthMatch = matches rowIdx expectedWidth row => matches
    T|> (rowIdx + 1, expectedWidth, None)
    F|> (rowIdx + 1, expectedWidth, Some (RaggedRows rowIdx expectedWidth (__aivi_list_length row)))

type Bool -> Int -> Int -> (List A) -> (Int, Int, Option MatrixError)
func __aivi_matrix_validateSubsequentRow = isFirst rowIdx expectedWidth row => isFirst
    T|> __aivi_matrix_validateFirstRow rowIdx expectedWidth row
    F|> __aivi_matrix_validateLengthMatch (__aivi_list_length row == expectedWidth) rowIdx expectedWidth row

type (Option MatrixError) -> Int -> Int -> (List A) -> (Int, Int, Option MatrixError)
func __aivi_matrix_validateRow = prevError rowIdx expectedWidth row => prevError
    ||> Some e -> (rowIdx + 1, expectedWidth, Some e)
    ||> None -> __aivi_matrix_validateSubsequentRow (rowIdx == 0) rowIdx expectedWidth row

type (Int, Int, Option MatrixError) -> (List A) -> (Int, Int, Option MatrixError)
func __aivi_matrix_fromRowsStep = state row => state
    ||> (rowIdx, width, error) -> __aivi_matrix_validateRow error rowIdx width row

type (Option MatrixError) -> Int -> Int -> (List (List A)) -> (Result MatrixError (Matrix A))
func __aivi_matrix_fromRowsDecide = error rowCount width inputRows => error
    ||> Some e -> Err e
    ||> None   -> Ok (MkMatrix width rowCount inputRows)

type (List (List A)) -> (Int, Int, Option MatrixError) -> (Result MatrixError (Matrix A))
func __aivi_matrix_fromRowsFinish = inputRows state => state
    ||> (rowCount, width, error) -> __aivi_matrix_fromRowsDecide error rowCount width inputRows

type (List (List A)) -> (Result MatrixError (Matrix A))
func __aivi_matrix_fromRows = inputRows => inputRows
    |> reduce __aivi_matrix_fromRowsStep (0, 0, None)
    |> __aivi_matrix_fromRowsFinish inputRows

type (Int -> Int -> A) -> Int -> Int -> A
func __aivi_matrix_initCellAt = build y x =>
    build x y

type Int -> (Int -> Int -> A) -> Int -> (List A)
func __aivi_matrix_buildRow = width build y =>
    __aivi_list_map (__aivi_matrix_initCellAt build y) (__aivi_list_range width)

type Int -> Int -> (Int -> Int -> A) -> (List (List A))
func __aivi_matrix_buildRows = width height build =>
    __aivi_list_map (__aivi_matrix_buildRow width build) (__aivi_list_range height)

type Int -> Int -> (Int -> Int -> A) -> Result MatrixError (Matrix A)
func __aivi_matrix_initHeight = width height build => height < 0
    T|> Err (NegativeHeight height)
    F|> Ok (MkMatrix width height (__aivi_matrix_buildRows width height build))

type Int -> Int -> (Int -> Int -> A) -> Result MatrixError (Matrix A)
func __aivi_matrix_init = width height build => width < 0
    T|> Err (NegativeWidth width)
    F|> __aivi_matrix_initHeight width height build

type A -> Int -> Int -> A
func __aivi_matrix_filledCell = value x y =>
    value

type Int -> Int -> A -> Result MatrixError (Matrix A)
func __aivi_matrix_filled = w h value =>
    __aivi_matrix_init w h (__aivi_matrix_filledCell value)

type (A -> Bool) -> Int -> A -> Int
func __aivi_matrix_countCell = predicate total item => predicate item
    T|> total + 1
    F|> total

type (A -> Bool) -> Int -> (List A) -> Int
func __aivi_matrix_countRow = predicate total row =>
    reduce (__aivi_matrix_countCell predicate) total row

type (A -> Bool) -> Matrix A -> Int
func __aivi_matrix_count = predicate matrix =>
    reduce (__aivi_matrix_countRow predicate) 0 (__aivi_matrix_rows matrix)

type (A -> Bool) -> A -> (Option A)
func __aivi_list_findTry = predicate item => predicate item
    T|> Some item
    F|> None

type (A -> Bool) -> (Option A) -> A -> (Option A)
func __aivi_list_findStep = predicate acc item => acc
    ||> Some v -> Some v
    ||> None   -> __aivi_list_findTry predicate item

type (A -> Bool) -> (List A) -> (Option A)
func __aivi_list_find = predicate items => items
    |> reduce (__aivi_list_findStep predicate) None

type Int -> Int -> (List A) -> A -> (Int, List A)
func __aivi_list_takeHelp = n count acc item => count >= n
    T|> (count, acc)
    F|> (count + 1, append acc [item])

type Int -> (Int, List A) -> A -> (Int, List A)
func __aivi_list_takeStep = n state item => state
    ||> (count, acc) -> __aivi_list_takeHelp n count acc item

type (Int, List A) -> (List A)
func __aivi_list_takeExtract = state => state
    ||> (_, acc) -> acc

type Int -> (List A) -> (List A)
func __aivi_list_take = n items => items
    |> reduce (__aivi_list_takeStep n) (0, [])
    |> __aivi_list_takeExtract

type (A -> A -> Bool) -> A -> A -> (List A) -> (Bool, List A)
func __aivi_list_sortByInsertFalse = cmp newItem current acc => cmp newItem current
    T|> (True, append (append acc [newItem]) [current])
    F|> (False, append acc [current])

type (A -> A -> Bool) -> A -> (Bool, List A) -> A -> (Bool, List A)
func __aivi_list_sortByInsertStep = cmp newItem state current => state
    ||> (True, acc) -> (True, append acc [current])
    ||> (False, acc) -> __aivi_list_sortByInsertFalse cmp newItem current acc

type (A -> A -> Bool) -> A -> (Bool, List A) -> (List A)
func __aivi_list_sortByInsertFinish = cmp newItem state => state
    ||> (True, result) -> result
    ||> (False, acc) -> append acc [newItem]

type (A -> A -> Bool) -> A -> (List A) -> (List A)
func __aivi_list_sortByInsert = cmp newItem sorted => sorted
    |> reduce (__aivi_list_sortByInsertStep cmp newItem) (False, [])
    |> __aivi_list_sortByInsertFinish cmp newItem

type (A -> A -> Bool) -> (List A) -> A -> (List A)
func __aivi_list_sortByStep = cmp sorted item =>
    __aivi_list_sortByInsert cmp item sorted

type (A -> A -> Bool) -> (List A) -> (List A)
func __aivi_list_sortBy = cmp items => items
    |> reduce (__aivi_list_sortByStep cmp) []

type Ord A => A -> A -> (List A) -> (Bool, List A)
func __aivi_list_insertSortedOrdFalse = newItem current acc =>
    __aivi_binary_lt newItem current
      T|> (True, append (append acc [newItem]) [current])
      F|> (False, append acc [current])

type Ord A => A -> (Bool, List A) -> A -> (Bool, List A)
func __aivi_list_insertSortedOrdStep = newItem state current => state
    ||> (True, acc) -> (True, append acc [current])
    ||> (False, acc) -> __aivi_list_insertSortedOrdFalse newItem current acc

type Ord A => A -> (Bool, List A) -> (List A)
func __aivi_list_insertSortedOrdFinish = newItem state => state
    ||> (True, result) -> result
    ||> (False, acc) -> append acc [newItem]

type Ord A => A -> (List A) -> (List A)
func __aivi_list_insertSortedOrd = newItem sorted => sorted
    |> reduce (__aivi_list_insertSortedOrdStep newItem) (False, [])
    |> __aivi_list_insertSortedOrdFinish newItem

type Ord A => (List A) -> A -> (List A)
func __aivi_list_sortOrdStep = sorted item =>
    __aivi_list_insertSortedOrd item sorted

type Ord A => (List A) -> (List A)
func sort = items =>
    items
      |> reduce __aivi_list_sortOrdStep []

type Text -> Bool
func __aivi_text_isEmpty = text => text == ""

type Text -> Bool
func __aivi_text_nonEmpty = text => text == ""
    T|> False
    F|> True

domain Duration over Int = {
    suffix ms : Int = value => Duration value
}

domain Retry over Int = {
    suffix times : Int = value => Retry value
}

type (A, B) -> A
func __aivi_pair_first = pair => pair
    ||> (a, _) -> a

type (A, B) -> B
func __aivi_pair_second = pair => pair
    ||> (_, b) -> b

type (A, B) -> (B, A)
func __aivi_pair_swap = pair => pair
    ||> (a, b) -> (b, a)

type (A -> C) -> (A, B) -> (C, B)
func __aivi_pair_mapFirst = transform pair => pair
    ||> (a, b) -> (transform a, b)

type (B -> C) -> (A, B) -> (A, C)
func __aivi_pair_mapSecond = transform pair => pair
    ||> (a, b) -> (a, transform b)

type (A -> C) -> (B -> D) -> (A, B) -> (C, D)
func __aivi_pair_mapBoth = transformFst transformSnd pair => pair
    ||> (a, b) -> (transformFst a, transformSnd b)

type A -> B -> (A, B)
func __aivi_pair_fromPair = a b => (a, b)

type A -> (A, A)
func __aivi_pair_duplicate = item => (item, item)

"#;

const MAX_COMPILE_TIME_RANGE_ELEMENTS: u64 = 4096;
