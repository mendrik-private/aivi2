# aivi.async

Lifecycle tracker for `Result`-producing signals. Wraps the raw `Result E A` stream from any
async source (HTTP, file reads, D-Bus, database queries) into a record with three observable
projections: `pending`, `done`, and `error`.

Once you have a `Signal (AsyncTracker E A)`, the projections become signals themselves — no
extra derivations needed:

```aivi
```

## Import

```aivi
use aivi.async (
    AsyncTracker
    step
    isPending
    isDone
    isFailed
)
```

---

## AsyncTracker

```aivi
type AsyncTracker E A = {
    pending: Bool,
    done: Option A,
    error: Option E
}
```

| Field | Type | Meaning |
| --- | --- | --- |
| `pending` | `Bool` | `True` while the first result has not yet arrived |
| `done` | `Option A` | `Some last-successful-value` once at least one `Ok` has arrived; `None` before that |
| `error` | `Option E` | `Some err` when the most recent result was `Err`; `None` otherwise |

**Stale-while-revalidate:** when a new `Err` arrives after a previous `Ok`, `done` keeps the
last successful value. This lets the UI keep showing useful data while surfacing the new error.

---

## step

Accumulation step function. Use it with `+|>` to turn a `Result`-producing signal into an
`AsyncTracker` signal.

**Type:** `AsyncTracker E A -> Result E A -> AsyncTracker E A`

```aivi
use aivi.async (
    AsyncTracker
    step
)

use aivi.http (
    HttpError
    HttpSource
)

type User = {
    id: Int,
    name: Text
}

@source http "https://api.example.com"
signal api : HttpSource

signal rawUsers : Signal (Result HttpError (List User)) = api.get "/users"

value initialUsers : AsyncTracker HttpError (List User) = {
    pending: True,
    done: None,
    error: None
}

signal users : Signal (AsyncTracker HttpError (List User)) = rawUsers
 +|> initialUsers step
```

The three projections are now independent reactive signals:

```aivi
// Spinner visible while loading
signal loading = users.pending

signal userList = users.done
signal fetchError = users.error
```

---

## isPending

Returns `True` while the first result has not arrived.

**Type:** `AsyncTracker E A -> Bool`

```aivi
use aivi.async (
    AsyncTracker
    isPending
)

type AsyncTracker Text Int -> Bool
func checkPending = tracker =>
    isPending tracker
```

---

## isDone

Returns `True` when at least one successful result has arrived.

**Type:** `AsyncTracker E A -> Bool`

```aivi
use aivi.async (
    AsyncTracker
    isDone
)

type AsyncTracker Text Int -> Bool
func checkDone = tracker =>
    isDone tracker
```

---

## isFailed

Returns `True` when the most recent result was a failure.

**Type:** `AsyncTracker E A -> Bool`

```aivi
use aivi.async (
    AsyncTracker
    isFailed
)

type AsyncTracker Text Int -> Bool
func checkFailed = tracker =>
    isFailed tracker
```

---

## Full UI example

```aivi
use aivi.async (
    AsyncTracker
    step
)

use aivi.http (
    HttpError
    HttpSource
)

type User = {
    id: Int,
    name: Text
}

@source http "https://api.example.com"
signal api : HttpSource

signal rawUsers : Signal (Result HttpError (List User)) = api.get "/users"

value initialUsers : AsyncTracker HttpError (List User) = {
    pending: True,
    done: None,
    error: None
}

signal users : Signal (AsyncTracker HttpError (List User)) = rawUsers
 +|> initialUsers step

value main =
    <Window title="Users">
        <Box>
            <Spinner />
            <Box />
            <Label text="Failed to load" />
            <Label text="No data yet" />
            <Label text="{items}" />
        </Box>
    </Window>

export main
```

---

## Fire-once idiom

There is no dedicated `do once` primitive today, but the accumulation operator gives you the
same behaviour. The pattern: keep a `Bool` that flips to `True` when the condition is first met
and never returns to `False`.

```aivi
use aivi.async (
    AsyncTracker
    step
)

use aivi.http (
    HttpError
    HttpSource
)

type User = {
    id: Int,
    name: Text
}

@source http "https://api.example.com"
signal api : HttpSource

signal rawUsers : Signal (Result HttpError (List User)) = api.get "/users"

value initialUsers : AsyncTracker HttpError (List User) = {
    pending: True,
    done: None,
    error: None
}

signal users : Signal (AsyncTracker HttpError (List User)) = rawUsers
 +|> initialUsers step

type Bool -> Option (List User) -> Bool
func trackFirstLoad = hasFired newDone => hasFired
 T|> True
 F|> isSome newDone

signal firstLoadDone : Signal Bool = users.done
 +|> False trackFirstLoad
```

`firstLoadDone` is a `Signal Bool` that is `False` until the first successful result arrives,
then becomes `True` permanently. Use it with `activeWhen` on a follow-up source to gate a
side-effect to fire only once.
