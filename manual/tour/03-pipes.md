# Pipes

Pipes are AIVI's primary control-flow surface. The value on the left becomes the current subject of the stage on the right.

## Transform, gate, branch, and pipe-call

The current subject placeholder is `.`. `_` is for wildcard patterns, not pipe subjects.

```aivi
type User = {
    active: Bool,
    age: Int,
    email: Text
}

type Shipping = { status: Text }

type Order = {
    shipping: Shipping
}

type Status =
  | Paid
  | Pending
  | Failed Text

fun maybeActiveUser:Option User user:User =>
    user
     ?|> (.active and .age > 18)

fun statusLabel:Text status:Status =>
    status
     ||> Paid          => "paid"
     ||> Pending       => "pending"
     ||> Failed reason => "failed {reason}"

fun startOrWait:Text ready:Bool =>
    ready
     T|> "start"
     F|> "wait"

fun observeShipping:Text shipping:Shipping =>
    shipping
     |> .status

fun shippingStatus:Text order:Order =>
    order
     |> .shipping
     | observeShipping
     |> .status
```

`|` performs pipe-call composition: it feeds the current subject into the named function on the right.

## Fan-out and fan-in

Use `*|>` to map across list-like carriers and `<|*` to join them back down.

```aivi
type User = {
    active: Bool,
    email: Text
}

fun joinEmails:Text items:List Text =>
    "joined"

val users: List User = [
    {
        active: True,
        email: "ada@example.com"
    }
]

val emails: List Text =
    users
     *|> .email

val joinedEmails: Text =
    users
     *|> .email
     <|* joinEmails

sig liveUsers: Signal (List User) = [
    {
        active: True,
        email: "ada@example.com"
    }
]

sig liveEmails: Signal (List Text) =
    liveUsers
     *|> .email

sig liveJoinedEmails: Signal Text =
    liveUsers
     *|> .email
     <|* joinEmails
```

## Applicative clusters

`&|>` gathers independent carriers so a final stage can apply a constructor or named function to all of them.

```aivi
type UserDraft =
  | UserDraft Text Text Int

type NamePair =
  | NamePair Text Text

sig nameText = "Ada"
sig emailText = "ada@example.com"
sig ageValue = 36
sig firstName = "Ada"
sig lastName = "Lovelace"

sig validatedUser =
  &|> nameText
  &|> emailText
  &|> ageValue
  |> UserDraft

sig namePair =
  &|> firstName
  &|> lastName
  |> NamePair
```

## Explicit recurrence pipes

`@|>` starts a recurrence, `?|>` guards it, and `<|@` advances it.

```aivi
domain Duration over Int
    literal s: Int -> Duration

type Cursor = { hasNext: Bool }

fun keep:Cursor cursor:Cursor =>
    cursor

val initial: Cursor = {
    hasNext: True
}

@recur.timer 1s
sig cursor: Signal Cursor =
    initial
     @|> keep
     ?|> .hasNext
     <|@ keep
```

There is no `#name` pipe memo syntax in the shipped language. Keep pipe stages to named functions, constructors, literals, projections, and the documented pipe operators.
