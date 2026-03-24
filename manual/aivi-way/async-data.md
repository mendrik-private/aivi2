# Async Data

Fetching data from an API is one of the first things most apps need to do.
In AIVI, an HTTP response is just another event that drives a signal.
There is no `async`/`await`, no `.then()`, no `Promise`.

## The pattern

```
@source http.get → Signal (Result HttpError Data) → ||> Ok/Err → markup
```

1. Declare a signal with `@source http.get`.
2. The signal holds `Result HttpError Data` — either `Ok` the parsed response or `Err` an error.
3. Use `\|\|>` or `T\|>`/`F\|>` to branch on the result.
4. Bind the branched signals to markup.

## A complete example

Fetching a user profile:

```text
// TODO: add a verified AIVI example here
```

## Handling the loading state

When you want an explicit `Loading` state, model it in the signal that consumes the HTTP result:

```text
// TODO: add a verified AIVI example here
```

## Retrying on error

```text
// TODO: add a verified AIVI example here
```

Passing `refreshOn: retryClicked` tells the source to re-fetch when `retryClicked` fires.

## Chaining requests

When a second request depends on the result of a first, start by extracting the `Ok` value into
its own signal:

```text
// TODO: add a verified AIVI example here
```

`userId` holds `Some id` when the user loaded successfully and `None` on error.
A later source can depend on `userId` the same way other sources depend on ordinary values.

## Why this is better than callbacks

In callback-based code, each step nests inside the previous one:

```
// typical callback hell (pseudo-code)
fetchUser(id, (err, user) => {
  if (err) { showError(err); return }
  fetchPosts(user.id, (err, posts) => {
    if (err) { showError(err); return }
    renderPosts(posts)
  })
})
```

In AIVI, the dependency is declared, not nested:

```text
// TODO: add a verified AIVI example here
```

Each step is a separate named signal. No nesting, no error routing, no lifecycle cleanup.

## Summary

- `@source http.get "url"` produces a `Signal (Result HttpError T)`.
- Map the result through `||>` arms for `Ok` and `Err`.
- Use `LoadState A` or similar to represent `Loading` / `Loaded` / `Failed`.
- Extract `Ok` values into their own signals before wiring dependent work.
- Retry by passing a click signal to `refreshOn` in the source options.
