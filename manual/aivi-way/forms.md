# Forms

Forms are a classic source of complexity in UI code: each field has its own state, validation
must run at the right time, and the submit button should only be active when everything is valid.

In AIVI, each field is a signal, validation is a gate (`?|>`), and the combined form state
is a derived signal.

## One signal per field

Declare a signal for each form field, driven by an `@source input.changed` event:

```text
-- declare 'rawName' driven by text-input changes from the "name-field" widget
-- declare 'rawEmail' driven by text-input changes from the "email-field" widget
-- declare 'rawBio' driven by text-input changes from the "bio-field" widget
```

Each source fires whenever the user types in the corresponding input widget.

## Validating with ?|>

`?|>` is the gate pipe: the value passes through only when the predicate is `True`.
A validated signal only has a value when the field is valid.

The gate predicate must be a named function — not a lambda:

```text
-- declare a predicate 'isValidEmail' that checks a text value contains "@"
-- declare a predicate 'isNonEmpty' that checks a text value is not empty
-- derive 'validName' from rawName, suppressing the value when the name is empty
-- derive 'validEmail' from rawEmail, suppressing when empty or when it lacks "@"
-- validEmail only carries a value when both predicates pass
```

`validName` only has a value when `rawName` is non-empty.
`validEmail` only has a value when `rawEmail` is non-empty AND contains `@`.

When a signal has no value (because a gate suppressed it), downstream signals depending on it
also have no value.

## Combining fields into a form signal

`&|>` is the applicative pipe — it combines independent signals under one applicative carrier.
`Signal` is applicative, not monadic: `&|>` does **not** bind the unwrapped value into a lambda.
Instead, stack the signals with `&|>` and then apply a pure constructor function:

```text
-- declare a product type 'ProfileForm' with text fields name, email, and bio
-- declare a constructor function 'makeProfile' combining three text values into a ProfileForm
-- combine validName, validEmail, and validBio signals applicatively
-- apply makeProfile to produce 'validForm', which only has a value when all three fields are valid
```

`validForm` only has a value when all three fields are valid simultaneously.
`makeProfile` receives the unwrapped `Text` values from each validated signal in declaration order.

## Enabling the submit button

```text
-- declare a function 'bothTrue' returning True only when both boolean arguments are True
-- derive 'nameValid' as True when rawName is non-empty
-- derive 'emailValid' as True when rawEmail is a valid email
-- combine nameValid and emailValid applicatively, applying bothTrue to get 'canSubmit'
-- canSubmit is True when both fields pass validation simultaneously
```

`canSubmit` is `True` when both fields are valid. Bind it to the button's `sensitive` attribute
so the button enables itself the moment both fields pass validation.

## Wiring submission

```text
-- declare 'submitClicked' driven by clicks on the "submit" button
-- validForm already guarantees validity by construction
-- in a real app, validForm would feed into an HTTP post source
```

## Full example

```text
-- declare a product type 'ContactForm' with text fields name and message
-- declare a predicate 'isNonEmpty' checking text is not empty
-- declare a predicate 'isLongEnough' checking text is longer than 10 characters
-- declare a constructor 'makeContact' combining name and message into a ContactForm
-- declare a function 'bothTrue' returning True only when both arguments are True
-- bind 'rawName' to text-input changes from the "name-input" widget
-- bind 'rawMessage' to text-input changes from the "message-input" widget
-- derive 'validName' from rawName, gating on isNonEmpty
-- derive 'validMessage' from rawMessage, gating on isLongEnough
-- combine validName and validMessage applicatively to produce 'validForm'
-- derive 'nameValid' as a Bool indicating whether the name passes validation
-- derive 'msgValid' as a Bool indicating whether the message passes validation
-- combine nameValid and msgValid to produce 'canSubmit'
-- bind 'submitClicked' to clicks on the "submit" button
-- render a Window titled "Contact" with a vertical Box
--   containing a name Entry, a message Entry, and a Send Button
--   the Send Button is enabled only when canSubmit is True
-- export main as the application entry point
```

The `sensitive` attribute on `<Button>` controls whether it is clickable.
It is bound to `canSubmit`, so the button enables itself the moment both fields are valid.

## Summary

- One `sig` per field, driven by `@source input.changed`.
- Gate predicates must be named functions; use `?|> isNonEmpty`, not `?|> \t => t != ""`.
- Combine validated fields with `&|>` and a pure constructor — `Signal` is applicative, not monadic.
- `canSubmit` is a derived boolean signal built with `&|>` and a combining function.
- Bind `sensitive={canSubmit}` to the submit button.
