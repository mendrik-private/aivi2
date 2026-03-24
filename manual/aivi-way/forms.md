# Forms

Forms are a classic source of complexity in UI code: each field has its own state, validation
must run at the right time, and the submit button should only be active when everything is valid.

In AIVI, each field is a signal, validation is a gate (`?|>`), and the combined form state
is a derived signal.

## One signal per field

Declare a signal for each form field, driven by an `@source input.changed` event:

```aivi
@source input.changed "name-field"
sig rawName : Signal Text

@source input.changed "email-field"
sig rawEmail : Signal Text

@source input.changed "bio-field"
sig rawBio : Signal Text
```

Each source fires whenever the user types in the corresponding input widget.

## Validating with ?|>

`?|>` is the gate pipe: the value passes through only when the predicate is `True`.
A validated signal only has a value when the field is valid.

The gate predicate must be a named function — not a lambda:

```aivi
fun isValidEmail:Bool #email:Text =>
    email != ""

fun isNonEmpty:Bool #text:Text =>
    text != ""

@source input.changed "name-field"
sig rawName : Signal Text

@source input.changed "email-field"
sig rawEmail : Signal Text

sig validName : Signal Text =
    rawName
     ?|> isNonEmpty

sig validEmail : Signal Text =
    rawEmail
     ?|> isNonEmpty
     ?|> isValidEmail
```

`validName` only has a value when `rawName` is non-empty.
`validEmail` only has a value when `rawEmail` is non-empty and valid.

When a signal has no value (because a gate suppressed it), downstream signals depending on it
also have no value.

## Combining fields into a form signal

`&|>` is the applicative pipe — it combines independent signals under one applicative carrier.
`Signal` is applicative, not monadic: `&|>` does **not** bind the unwrapped value into a lambda.
Instead, stack the signals with `&|>` and then apply a pure constructor function:

```aivi
fun isNonEmpty:Bool #text:Text =>
    text != ""

fun isValidEmail:Bool #email:Text =>
    email != ""

type ProfileForm =
  | ProfileForm Text Text Text

@source input.changed "name-field"
sig rawName : Signal Text

@source input.changed "email-field"
sig rawEmail : Signal Text

@source input.changed "bio-field"
sig rawBio : Signal Text

sig validName : Signal Text =
    rawName
     ?|> isNonEmpty

sig validEmail : Signal Text =
    rawEmail
     ?|> isNonEmpty
     ?|> isValidEmail

sig validBio : Signal Text =
    rawBio
     ?|> isNonEmpty

sig validForm : Signal ProfileForm =
  &|> validName
  &|> validEmail
  &|> validBio
  |> ProfileForm
```

`validForm` only has a value when all three fields are valid simultaneously.
The constructor receives the unwrapped `Text` values from each validated signal in declaration order.

## Enabling the submit button

```aivi
fun isNonEmpty:Bool #text:Text =>
    text != ""

fun isValidEmail:Bool #email:Text =>
    email != ""

fun bothTrue:Bool #a:Bool #b:Bool =>
    a and b

@source input.changed "name-field"
sig rawName : Signal Text

@source input.changed "email-field"
sig rawEmail : Signal Text

sig nameValid : Signal Bool =
    rawName
     |> isNonEmpty

sig emailValid : Signal Bool =
    rawEmail
     |> isValidEmail

sig canSubmit : Signal Bool =
  &|> nameValid
  &|> emailValid
  |> bothTrue
```

`canSubmit` is `True` when both fields are valid. Bind it to the button's `sensitive` attribute
so the button enables itself the moment both fields pass validation.

## Wiring submission

```aivi
sig submitClicked : Signal Unit
```

`submitClicked` is an input signal. In markup, connect it with `onClick={submitClicked}` on the
submit button.

## Full example

```aivi
type Orientation =
  | Vertical
  | Horizontal

type ContactForm =
  | ContactForm Text Text

fun isNonEmpty:Bool #text:Text =>
    text != ""

fun isLongEnough:Bool #text:Text =>
    text != ""

fun makeContact:ContactForm #name:Text #message:Text =>
    ContactForm name message

fun bothTrue:Bool #a:Bool #b:Bool =>
    a and b

@source input.changed "name-input"
sig rawName : Signal Text

@source input.changed "message-input"
sig rawMessage : Signal Text

sig validName : Signal Text =
    rawName
     ?|> isNonEmpty

sig validMessage : Signal Text =
    rawMessage
     ?|> isLongEnough

sig validForm : Signal ContactForm =
  &|> validName
  &|> validMessage
  |> makeContact

sig nameValid : Signal Bool =
    rawName
     |> isNonEmpty

sig msgValid : Signal Bool =
    rawMessage
     |> isLongEnough

sig canSubmit : Signal Bool =
  &|> nameValid
  &|> msgValid
  |> bothTrue

sig submitClicked : Signal Unit

val main =
    <Window title="Contact">
        <Box orientation={Vertical} spacing={8}>
            <Entry id="name-input" placeholderText="Name" />
            <Entry id="message-input" placeholderText="Message" />
            <Button label="Send" sensitive={canSubmit} onClick={submitClicked} />
        </Box>
    </Window>

export main
```

The `sensitive` attribute on `<Button>` controls whether it is clickable.
It is bound to `canSubmit`, so the button enables itself the moment both fields are valid.

## Summary

- One `sig` per field, driven by `@source input.changed`.
- Gate predicates must be named functions; use `?|> isNonEmpty`, not inline lambdas.
- Combine validated fields with `&|>` and a pure constructor — `Signal` is applicative, not monadic.
- `canSubmit` is a derived boolean signal built with `&|>` and a combining function.
- Bind `sensitive={canSubmit}` to the submit button.
