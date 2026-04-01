# aivi.validation

Utilities for working with `Validation E A` — like `Result`, but with an accumulation-oriented applicative path for independent failures. The current executable accumulation surface is `zipValidation`, which combines two `Validation (NonEmptyList E)` values and collects errors from both sides.

```aivi
use aivi.validation (
    Errors
    isValid
    isInvalid
    getOrElse
    mapErr
    toResult
    fromResult
    toOption
    map
    andThen
    zipValidation
    fold
)
```

---

## Errors

A type alias for a non-empty list of errors. Used as the error carrier in `zipValidation`.

```aivi
type Errors E =
  | NonEmptyList E
```

`Errors E` guarantees at least one error is present when a validation fails. Import `NonEmptyList` from `aivi.nonEmpty` to construct values of this type.

---

## isValid

Returns `True` if the validation holds a value.

**Type:** `opt:(Validation E A) -> Bool`

```aivi
use aivi.validation (isValid)

type Validation Text Int -> Bool
func passed = v =>
    isValid v
```

---

## isInvalid

Returns `True` if the validation has failed.

**Type:** `opt:(Validation E A) -> Bool`

```aivi
use aivi.validation (isInvalid)

type Validation Text Int -> Bool
func rejected = v =>
    isInvalid v
```

---

## getOrElse

Extracts the value from `Valid`, or returns the fallback if `Invalid`.

**Type:** `fallback:A -> opt:(Validation E A) -> A`

```aivi
use aivi.validation (getOrElse)

type Validation Text Int -> Int
func safeValue = v =>
    getOrElse 0 v
```

---

## mapErr

Transforms the error inside `Invalid`, leaving `Valid` untouched.

**Type:** `transform:(E1 -> E2) -> opt:(Validation E1 A) -> Validation E2 A`

```aivi
use aivi.validation (mapErr)

type Text -> Int
func toCode = message =>
    42

type Validation Text Int -> (Validation Int Int)
func withErrorCode = v => v
  |> mapErr toCode
```

---

## toResult

Converts a `Validation` to a `Result`. `Valid value` becomes `Ok value`; `Invalid err` becomes `Err err`.

**Type:** `opt:(Validation E A) -> Result E A`

```aivi
use aivi.validation (toResult)

type Validation Text Int -> (Result Text Int)
func asResult = v =>
    toResult v
```

---

## fromResult

Converts a `Result` to a `Validation`. `Ok value` becomes `Valid value`; `Err error` becomes `Invalid error`.

**Type:** `opt:(Result E A) -> Validation E A`

```aivi
use aivi.validation (fromResult)

type Result Text Int -> (Validation Text Int)
func asValidation = r =>
    fromResult r
```

---

## toOption

Converts a `Validation` to an `Option`, discarding any errors. `Valid value` becomes `Some value`; `Invalid` becomes `None`.

**Type:** `opt:(Validation E A) -> Option A`

```aivi
use aivi.validation (toOption)

type Validation Text Int -> (Option Int)
func justValue = v =>
    toOption v
```

---

## map

Transforms the value inside `Valid` using a function, leaving `Invalid` untouched.

**Type:** `transform:(A -> B) -> opt:(Validation E A) -> Validation E B`

```aivi
use aivi.validation (map)

type Int -> Int
func double = n =>
    n * 2

type Validation Text Int -> (Validation Text Int)
func doubleValid = v => v
  |> map double
```

---

## andThen

Chains a `Validation`-returning function over a `Valid` value. If the input is `Invalid`, the error is propagated without calling the function.

Unlike `zipValidation`, `andThen` stops at the first failure and does not accumulate errors.

**Type:** `next:(A -> Validation E B) -> opt:(Validation E A) -> Validation E B`

```aivi
use aivi.validation (andThen)

type Int -> (Validation Text Int)
func ensurePositive = n => n > 0
 T|> Valid n
 F|> Invalid "must be positive"

type Validation Text Int -> (Validation Text Int)
func validateCount = v => v
  |> andThen ensurePositive
```

---

## zipValidation

Combines two validations into one that holds a tuple of both values. If either or both sides are `Invalid`, **all** errors are accumulated into a `NonEmptyList`. This is the primary tool for parallel form validation.

**Type:** `left:(Validation (NonEmptyList E) A) -> right:(Validation (NonEmptyList E) B) -> Validation (NonEmptyList E) (A, B)`

```aivi
use aivi.validation (zipValidation)

use aivi.nonEmpty (
    NonEmptyList
    singleton
)

type FieldError =
  | EmptyName
  | InvalidAge

type Text -> (Validation (NonEmptyList FieldError) Text)
func validateName = name => name
 ||> "" -> Invalid (singleton EmptyName)
 ||> _  -> Valid name

type Int -> (Validation (NonEmptyList FieldError) Int)
func validateAge = age => age > 0
 T|> Valid age
 F|> Invalid (singleton InvalidAge)

type Text -> Int -> (Validation (NonEmptyList FieldError) (Text, Int))
func validateForm = name age =>
    zipValidation (validateName name) (validateAge age)
```

If both `validateName` and `validateAge` fail, the result is `Invalid` containing both `EmptyName` and `InvalidAge` in a single list.

---

## fold

Collapses a `Validation` to a single value by applying `onValid` to `Valid` or `onInvalid` to `Invalid`.

**Type:** `onInvalid:(E -> B) -> onValid:(A -> B) -> opt:(Validation E A) -> B`

```aivi
use aivi.validation (fold)

use aivi.nonEmpty (
    NonEmptyList
    length
)

type NonEmptyList Text -> Int
func countErrors = errors =>
    length errors

type Int -> Int
func identity = n =>
    n

type Validation (NonEmptyList Text) Int -> Int
func scoreOrErrorCount = v =>
    fold countErrors identity v
```
