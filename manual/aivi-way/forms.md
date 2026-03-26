# Forms

Keep form state in signals, then derive validated shapes from those signals. The current manual stays conservative and shows the data flow; event wiring belongs at the markup boundary.

```aivi
type Registration =
  | Registration Text Text Int

fun nonEmpty:Bool value:Text =>
    value != ""

fun positive:Bool value:Int =>
    value > 0

fun allReady:Bool nameReady:Bool emailReady:Bool ageReady:Bool =>
    nameReady and emailReady and ageReady

sig nameText = "Ada"
sig emailText = "ada@example.com"
sig ageValue = 36

sig draft: Signal Registration =
  &|> nameText
  &|> emailText
  &|> ageValue
  |> Registration

sig nameReady: Signal Bool =
    nameText
     |> nonEmpty

sig emailReady: Signal Bool =
    emailText
     |> nonEmpty

sig ageReady: Signal Bool =
    ageValue
     |> positive

sig canSubmit: Signal Bool =
  &|> nameReady
  &|> emailReady
  &|> ageReady
  |> allReady
```

A good form flow is:

1. source or event signals own raw field values
2. pure helpers validate or normalize them
3. applicative clusters build the checked aggregate
4. markup reads the derived signals

Avoid inventing `input.changed`-style providers unless they are documented in `aivi.md` and exercised by the shipped runtime surface.
