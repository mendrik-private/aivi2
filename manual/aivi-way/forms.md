# Forms

Forms are a classic source of complexity in UI code: each field has its own state, validation
must run at the right time, and the submit button should only be active when everything is valid.

In AIVI, each field is a signal, validation is a gate (`?|>`), and the combined form state
is a derived signal.

## One signal per field

Declare a signal for each form field, driven by an `@source input.changed` event:

```text
// TODO: add a verified AIVI example here
```

Each source fires whenever the user types in the corresponding input widget.

## Validating with ?|>

`?|>` is the gate pipe: the value passes through only when the predicate is `True`.
A validated signal only has a value when the field is valid.

The gate predicate must be a named function — not a lambda:

```text
// TODO: add a verified AIVI example here
```

`validName` only has a value when `rawName` is non-empty.
`validEmail` only has a value when `rawEmail` is non-empty and the email field is present.

When a signal has no value (because a gate suppressed it), downstream signals depending on it
also have no value.

## Combining fields into a form signal

`&|>` is the applicative pipe — it combines independent signals under one applicative carrier.
`Signal` is applicative, not monadic: `&|>` does **not** bind the unwrapped value into a lambda.
Instead, stack the signals with `&|>` and then apply a pure constructor function:

```text
// TODO: add a verified AIVI example here
```

`validForm` only has a value when all three fields are valid simultaneously.
The constructor receives the unwrapped `Text` values from each validated signal in declaration order.

## Enabling the submit button

```text
// TODO: add a verified AIVI example here
```

`canSubmit` is `True` when both fields are valid. Bind it to the button's `sensitive` attribute
so the button enables itself the moment both fields pass validation.

## Wiring submission

```text
// TODO: add a verified AIVI example here
```

`submitClicked` is an input signal. In markup, connect it with `onClick={submitClicked}` on the
submit button.

## Full example

```text
// TODO: add a verified AIVI example here
```

The `sensitive` attribute on `<Button>` controls whether it is clickable.
It is bound to `canSubmit`, so the button enables itself the moment both fields are valid.

## Summary

- One `sig` per field, driven by `@source input.changed`.
- Gate predicates must be named functions; use `?|> isNonEmpty`, not inline lambdas.
- Combine validated fields with `&|>` and a pure constructor — `Signal` is applicative, not monadic.
- `canSubmit` is a derived boolean signal built with `&|>` and a combining function.
- Bind `sensitive={canSubmit}` to the submit button.
