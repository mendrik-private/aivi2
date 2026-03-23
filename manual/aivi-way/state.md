# State

AIVI programs can have state at two levels: **local** (owned by one component) and **shared**
(owned by the application domain and accessible across components).

## Local state with sig

A `sig` declared alongside a component's markup is local to that component.
It is created when the component mounts and destroyed when it unmounts.

A counter is the canonical example of local state:

```text
-- declare a local signal 'count' starting at 0
-- bind 'increment' to the "increment" button click, updating count by adding 1
-- bind 'decrement' to the "decrement" button click, updating count by subtracting 1
-- derive 'label' as the text representation of count
-- render a Window titled "Counter" with a vertical Box
--   containing a Label bound to label, a "+" Button, and a "−" Button
-- export main as the application entry point
```

`count` starts at `0`. `@|>` opens the recurrence on `increment`; `<|@` adds `decrement` as
a second recurse point. Each event folds through `update`. Nothing else in the application
can see or modify `count`.

## When to use local state

Use a local `sig` when:

- The state is only relevant to one part of the UI.
- No other component needs to read or write it.
- The state should reset when the component is removed (e.g., a modal's open/closed state).

Examples: accordion open/closed, tooltip visibility, input focus, scroll position.

## Shared state with domain

When state needs to be accessible from multiple parts of the UI, it belongs in a `domain`:

```text
-- declare a domain 'AppState' holding application-wide signals
-- 'currentUser' starts as None (no logged-in user)
-- 'theme' starts as Light
-- 'notifications' starts as an empty list
```

A `domain` is a named collection of signals. Any component can read from a domain signal.
Only the domain itself (or providers) can write to it.

## Reading from a domain

```text
-- derive 'headerUser' by reading AppState.currentUser
-- if a user is logged in, use their name
-- if no user, use the text "Guest"
```

The dot notation `AppState.currentUser` reads a signal from the domain.

## Writing to a domain

Domain signals accept updates through `provider` declarations:

```text
-- declare a provider 'LoginProvider' that writes to the AppState domain
-- bind 'loginResult' to the result of an HTTP POST to "/api/login"
-- when the login succeeds, set AppState.currentUser to Some user
-- when the login fails, set AppState.currentUser to None
```

Providers are the only mechanism for writing to domain state.
This keeps updates centralized and auditable.

## When to use domain state

Use domain state when:

- Multiple components read the same value (e.g., the current user's name in a header and a profile page).
- State must survive component unmount (e.g., a shopping cart that persists while navigating).
- You want a single source of truth for application-wide data.

## Avoiding over-sharing

Not every signal needs to be in a domain. Start with local state and only promote to domain
state when you actually need it in two or more places.

Over-shared state makes programs harder to understand because the number of things that can
change a value grows. Local state's value can only change via its own declared recurrence.

## Comparison

| | Local `sig` | Domain `sig` |
|---|---|---|
| Scope | Component | Application |
| Lifetime | Component lifetime | Application lifetime |
| Who can write | Declared recurrence | Provider declarations |
| When to use | One component needs it | Multiple components or persists across navigation |

## Summary

- Local `sig` is scoped to one component and resets on unmount.
- `domain` holds application-wide signals readable by any component.
- `provider` is the only way to update domain signals.
- Start local; promote to domain when needed.
