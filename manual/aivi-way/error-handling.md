# Error Handling

AIVI uses types, not ambient exceptions.

- `Option A` means a value may be absent.
- `Result E A` means an effect or computation can fail with `E`.
- `Validation E A` is for checked data where invalid states should stay explicit.

## Picking the right carrier

```aivi
use aivi (
    Err
    Invalid
    None
    Option
    Result
    Valid
    Validation
)

use aivi.option (getOrElse)

use aivi.result (withDefault)

use aivi.validation (Errors)

val maybeName: Option Text = None
val name: Text = getOrElse "guest" maybeName
val count: Result Text Int = Err "offline"
val safeCount: Int = withDefault 0 count
val checked: Validation (Errors Text) Text = Valid "Ada"
```

## Branch explicitly

```aivi
use aivi (
    Err
    Invalid
    Ok
    Result
    Valid
    Validation
)

fun resultLabel:Text status:(Result Text Text) =>
    status
     ||> Ok body     => body
     ||> Err message => message

fun validationLabel:Text status:(Validation Text Text) =>
    status
     ||> Valid body      => body
     ||> Invalid message => message
```

Prefer precise error types over generic text until you reach a presentation boundary.
