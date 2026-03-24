# State

AIVI programs can have state at two levels: **local** (owned by one component) and **shared**
(used by multiple parts of the program).

## Local state with sig

A `sig` declared alongside a component's markup is local to that component.
It is created when the component mounts and destroyed when it unmounts.

A counter is the canonical example of local state:

```aivi
type Orientation =
  | Vertical
  | Horizontal

fun applyIncrement:Int #id:Text #n:Int =>
    id == "increment"
     T|> n + 1
     F|> n

fun applyDecrement:Int #id:Text #n:Int =>
    id == "decrement"
     T|> n - 1
     F|> n

fun updateFromButton:Int #id:Text #n:Int =>
    n
     |> applyIncrement id
     |> applyDecrement id

provider button.clicked
    wakeup: sourceEvent
    argument id: Text

@source button.clicked "increment"
@source button.clicked "decrement"
sig count : Signal Int =
    0
     @|> updateFromButton
     <|@ updateFromButton

sig label : Signal Text = "{count}"

val main =
    <Window title="Counter">
        <Box orientation={Vertical} spacing={8}>
            <Label text={label} />
            <Button id="increment" label="+" />
            <Button id="decrement" label="-" />
        </Box>
    </Window>

export main
```

`count` starts at `0`. The two button ids drive the recurrence, and `updateFromButton` inspects
the incoming id to decide whether to increment or decrement. Nothing else in the application can
see or modify `count` unless you explicitly derive another signal from it.

## When to use local state

Use a local `sig` when:

- The state is only relevant to one part of the UI.
- No other component needs to read or write it.
- The state should reset when the component is removed.

Examples: accordion open/closed, tooltip visibility, input focus, scroll position.

## Shared state as top-level signals

When state needs to be accessible from multiple parts of the UI, model it as named top-level
signals rather than inventing a separate mutable container:

```aivi
type Theme = Light | Dark

type User = {
    id: Int,
    name: Text
}

type Notification = {
    id: Int,
    message: Text
}

sig currentUser : Signal (Option User) = None
sig theme : Signal Theme = Light
sig notifications : Signal (List Notification) = []
```

These are shared because any other signal or markup binding can derive from them.
The current compiler does not have a separate `domain` state feature — plain top-level signals are
the right tool here.

## Reading shared state

```aivi
type User = {
    id: Int,
    name: Text
}

sig currentUser : Signal (Option User) = None

fun headerUserName:Text #user:(Option User) =>
    user
     ||> Some u => u.name
     ||> None   => "Guest"

sig headerUser : Signal Text =
    currentUser
     |> headerUserName
```

A header label, profile panel, and status bar can all derive their own views from the same shared
signal without mutating it directly.

## Updating shared state from sources

Shared state is still source-driven. Instead of "writing into a domain", derive the next shared
value from the result of a source:

```aivi
type HttpError = {
    message: Text,
    code: Int
}

type User = {
    id: Int,
    name: Text
}

type Credentials = {
    username: Text,
    password: Text
}

sig credentials : Signal Credentials = {
    username: "",
    password: ""
}

@source http.post "/api/login" with {
    body: credentials
}
sig loginResult : Signal (Result HttpError User)

fun userFromLogin:(Option User) #result:(Result HttpError User) =>
    result
     ||> Ok user => Some user
     ||> Err _   => None

sig currentUser : Signal (Option User) =
    loginResult
     |> userFromLogin
```

The source produces a `Result`, and the shared signal is just another pure transformation of that
source output.

## When to use shared state

Use shared state when:

- Multiple components read the same value.
- State must survive the lifetime of one particular view.
- You want one authoritative signal that other signals derive from.

## Avoiding over-sharing

Not every signal needs to be shared. Start with local state and only promote to a top-level signal
when you actually need it in two or more places.

Over-shared state makes programs harder to understand because the number of things that can
affect a value grows. Local state keeps the update path small and explicit.

## Comparison

| | Local `sig` | Shared top-level signal |
|---|---|---|
| Scope | One component | Whole program |
| Lifetime | Component lifetime | Program lifetime |
| How it updates | Local recurrence and attached sources | Top-level source-driven derivation |
| When to use | One component needs it | Multiple views depend on it |

## Summary

- Local `sig` is scoped to one component and resets on unmount.
- Shared state is modeled with top-level signals derived from sources.
- Read shared state by deriving more signals from it.
- Start local; promote to shared when multiple views depend on it.
