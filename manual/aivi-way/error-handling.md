# Error Handling

AIVI has no exceptions. Errors are values.

This is not a limitation — it is a design. When errors are values, the type system enforces
that you handle them. Nothing can go wrong silently.

## Result E A: the error type

```aivi
type Result E A = Ok A | Err E
```

A `Result E A` is either a successful value (`Ok A`) or an error (`Err E`).
Every operation that can fail returns `Result`.

## Matching on results

Use `\|\|>` to branch on `Ok` vs `Err`:

```aivi
fun describeResult:Text #result:(Result Text Int) =>
    result
     ||> Ok n    => "Success: {n}"
     ||> Err msg => "Failed: {msg}"
```

The compiler ensures you handle both cases. You cannot accidentally ignore an error.

## Chaining operations that might fail

A common pattern is a sequence of operations where each step can fail.
Use `||>` to branch on `Ok` and `Err` at each step:

```aivi
type User = {
    name: Text,
    age: Int
}

fun parseIntResult:(Result Text Int) #text:Text =>
    Ok 0

fun validateAge:(Result Text Int) #n:Int =>
    n > 0
     T|> Ok n
     F|> Err "Age must be positive"

fun checkedAge:(Result Text Int) #ageText:Text =>
    parseIntResult ageText
     ||> Ok age  => validateAge age
     ||> Err msg => Err msg

fun validateUser:(Result Text User) #name:Text #ageText:Text =>
    checkedAge ageText
     ||> Ok age  => Ok { name: name, age: age }
     ||> Err msg => Err msg
```

## Propagating errors in signals

When a signal holds a `Result`, downstream signals can propagate the `Ok` value or branch
on the `Err`:

```aivi
type HttpError = {
    message: Text,
    code: Int
}

type Profile = {
    name: Text,
    bio: Text
}

fun nameFromResult:Text #result:(Result HttpError Profile) =>
    result
     ||> Ok profile => profile.name
     ||> Err _      => "Unknown"

fun errorFromResult:(Option Text) #result:(Result HttpError Profile) =>
    result
     ||> Ok _    => None
     ||> Err err => Some err.message

@source http.get "/api/profile"
sig profileResult : Signal (Result HttpError Profile)

sig profileName : Signal Text =
    profileResult
     |> nameFromResult

sig profileError : Signal (Option Text) =
    profileResult
     |> errorFromResult
```

## Showing errors in markup

```aivi
type HttpError = {
    message: Text,
    code: Int
}

type Profile = {
    name: Text,
    bio: Text
}

type Orientation =
  | Vertical
  | Horizontal

fun nameFromResult:Text #result:(Result HttpError Profile) =>
    result
     ||> Ok profile => profile.name
     ||> Err _      => "Unknown"

fun errorFromResult:(Option Text) #result:(Result HttpError Profile) =>
    result
     ||> Ok _    => None
     ||> Err err => Some err.message

fun hasErrorMsg:Bool #err:(Option Text) =>
    err
     T|> True
     F|> False

fun errorText:Text #err:(Option Text) =>
    err
     ||> Some msg => msg
     ||> None     => ""

@source http.get "/api/profile"
sig profileResult : Signal (Result HttpError Profile)

sig profileName : Signal Text =
    profileResult
     |> nameFromResult

sig profileError : Signal (Option Text) =
    profileResult
     |> errorFromResult

sig hasError : Signal Bool =
    profileError
     |> hasErrorMsg

sig errText : Signal Text =
    profileError
     |> errorText

val main =
    <Window title="Profile">
        <Box orientation={Vertical} spacing={8}>
            <show when={hasError}>
                <Label text={errText} />
            </show>
            <Label text={profileName} />
        </Box>
    </Window>

export main
```

## The Option type for optional values

`Option A` handles absence (not failure):

```aivi
type Option A = Some A | None

type Item = {
    id: Int,
    name: Text
}

sig selectedItem : Signal (Option Item) = None

fun selectionLabel:Text #selected:(Option Item) =>
    selected
     ||> Some item => "Selected: {item.name}"
     ||> None      => "Nothing selected"

sig selectionText : Signal Text =
    selectedItem
     |> selectionLabel
```

Use `Result` when an operation attempted and failed.
Use `Option` when a value is simply optional.

## Never throw

There is no `throw` in AIVI. Functions that encounter error conditions return `Err msg`.
Callers handle it explicitly.

This means:
- Reading a source file: returns `Result Text`.
- Parsing a number: returns `Result Int`.
- HTTP requests: return `Result Response`.
- Looking up a key in a map: returns `Option Value`.

The return type tells you whether the operation can fail before you even read the documentation.

## Recovering from errors

To fall back to a default value when a result is an error:

```aivi
type HttpError = {
    message: Text,
    code: Int
}

type Profile = { name: Text }

fun withDefault:A #fallback:A #result:(Result HttpError A) =>
    result
     ||> Ok value => value
     ||> Err _    => fallback

fun profileNameOrAnon:Text #result:(Result HttpError Profile) =>
    withDefault { name: "Anonymous" } result
     |> .name
```

Or inline in a pipe:

```aivi
type HttpError = {
    message: Text,
    code: Int
}

type Profile = { name: Text }

fun nameOrFallback:Text #result:(Result HttpError Profile) =>
    result
     ||> Ok profile => profile.name
     ||> Err _      => "Anonymous"

@source http.get "/api/profile"
sig profileResult : Signal (Result HttpError Profile)

sig displayName : Signal Text =
    profileResult
     |> nameOrFallback
```

## Counting valid items in a list

When validating a list of items, derive a count of the valid values with a named predicate:

```aivi
use aivi.list (count)

fun isValidAge:Bool #n:Int =>
    n > 0 and n < 150

val ageInputs:List Int = [
    25,
    0,
    30,
    200,
    42
]

val validCount:Int =
    ageInputs
     |> count isValidAge
```

This gives you a stable summary signal or value you can render directly. For detailed error
reporting, branch on each item individually with `||>` in the calling code.

## Summary

- AIVI has no exceptions. Errors are `Result E A = Ok A | Err E`.
- Use `||>` to branch on `Ok` vs `Err`. The compiler enforces exhaustiveness.
- `Option A = Some A | None` for optional values.
- Chain results with `||>` arms that produce new `Result` values.
- `withDefault` recovers a fallback when a result is an error.
- Return type signatures communicate failure potential before reading docs.
