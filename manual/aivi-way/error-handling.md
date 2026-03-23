# Error Handling

AIVI has no exceptions. Errors are values.

This is not a limitation — it is a design. When errors are values, the type system enforces
that you handle them. Nothing can go wrong silently.

## Result E A: the error type

```text
-- declare the Result type: Ok carrying a success value of type A, or Err carrying an error of type E
```

A `Result E A` is either a successful value (`Ok A`) or an error (`Err E`).
Every operation that can fail returns `Result`.

## Matching on results

Use `\|\|>` to branch on `Ok` vs `Err`:

```text
-- declare a function 'describeResult' matching on a Result Text Int
-- when Ok, format the number as "Success: N"
-- when Err, format the message as "Failed: msg"
```

The compiler ensures you handle both cases. You cannot accidentally ignore an error.

## Chaining operations that might fail

A common pattern is a sequence of operations where each step can fail.
Use `||>` to branch on `Ok` and `Err` at each step:

```text
-- declare a function 'validateAge' that returns Ok n if n is between 1 and 149, otherwise Err with a message
-- declare a function 'validateUser' that parses raw input
--   convert ageText to an integer, returning Err if it is not a number
--   then validate the parsed integer with validateAge
--   if both succeed, return Ok with a User record containing name and age
```

## Propagating errors in signals

When a signal holds a `Result`, downstream signals can propagate the `Ok` value or branch
on the `Err`:

```text
-- bind 'profileResult' to an HTTP GET for the user profile, producing Ok Profile or Err HttpError
-- derive 'profileName': the user's name on success, "Unknown" on error
-- derive 'profileError': None on success, Some with the error message on error
```

## Showing errors in markup

```text
-- derive 'hasError' as True when profileError holds a message, False otherwise
-- derive 'errorText' as the error message when present, empty string otherwise
-- render a vertical Box
--   show an error Label with the error text only when hasError is True
--   always show a Label with the profile name
```

## The Option type for optional values

`Option A` handles absence (not failure):

```text
-- Option A is a sum type: Some (carrying A) or None
-- declare a signal 'selectedItem' of type Option Item
-- derive 'selectionLabel': show "Selected: name" when an item is selected, "Nothing selected" otherwise
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

```text
-- declare a generic function 'withDefault' that returns the Ok value on success, or the default on error
-- use withDefault to extract a name from profileResult, falling back to "Anonymous"
```

Or inline in a pipe:

```text
-- derive 'displayName' from profileResult: use the profile name on success, "Anonymous" on error
```

## Collecting errors from a list

When validating a list of items, validate each item independently and filter to keep only
the valid ones. Use named predicate functions with `List.filter`:

```text
-- declare a predicate 'isValidAge' returning True when an integer is between 1 and 149
-- derive 'validAges' from ageInputs by filtering out values that do not pass isValidAge
```

This keeps only the items that pass validation. For error reporting, match on each item
individually with `||>` in the calling code.

## Summary

- AIVI has no exceptions. Errors are `Result E A = Ok A | Err E`.
- Use `||>` to branch on `Ok` vs `Err`. The compiler enforces exhaustiveness.
- `Option A = Some A | None` for optional values.
- Chain results with `||>` arms that produce new `Result` values.
- `withDefault` recovers a fallback when a result is an error.
- Return type signatures communicate failure potential before reading docs.
