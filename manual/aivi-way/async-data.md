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
-- declare a product type 'User' with integer id and text fields name, email, bio
-- declare a parametric type 'LoadState A' with variants Loading, Loaded (carrying A), Failed (carrying error text)
-- bind 'userResponse' to an HTTP GET request for user 1, producing Ok User or Err HttpError
-- derive 'userState' by mapping Ok to Loaded and Err to Failed
-- derive 'userName': show user's name when loaded, "Loading…" when loading, "Unknown" on failure
-- derive 'userBio': show user's bio when loaded, empty string when loading, error message on failure
-- render a Window titled "User Profile" with a vertical Box
--   containing Labels bound to userName and userBio
-- export main as the application entry point
```

## Handling the loading state

The above example maps `Loading` to a placeholder string. For a proper loading spinner:

```text
-- derive 'isLoading' as True when userState is Loading, False otherwise
-- render a Window titled "Profile" with a vertical Box
-- show an active Spinner only while isLoading is True
-- show a Label with the user name below
```

## Retrying on error

```text
-- bind 'retryClicked' to clicks on the "retry" button
-- bind 'data' to an HTTP GET request that re-fetches whenever retryClicked fires
-- the signal carries either a Payload or an HttpError
```

Passing `refreshOn: retryClicked` tells the source to re-fetch when `retryClicked` fires.

## Chaining requests

When a second request depends on the result of a first, use `||>` to extract the `Ok` value.
The signal only produces a value when the result is `Ok`:

```text
-- bind 'userResult' to an HTTP GET for user 1
-- derive 'userId' as Some user's id when the user loaded successfully, or None on error
-- bind 'postsResult' to an HTTP GET for posts, passing userId as a query parameter
-- postsResult only fetches when userId has a value
```

`userId` holds `Some id` when the user loaded successfully and `None` on error.
The posts request can use `userId` as a reactive source argument.

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
-- derive userId from userResult: Some id on success, None on error
-- derive posts using userId as a dependency
-- derive view by rendering posts when they load successfully
-- each step is a separate named signal with no nesting
```

Each step is a separate named signal. No nesting, no error routing, no lifecycle cleanup.

## Summary

- `@source http.get "url"` produces a `Signal (Result HttpError T)`.
- Map the result through `||>` arms for `Ok` and `Err`.
- Use `LoadState A` or similar to represent `Loading` / `Loaded` / `Failed`.
- Extract `Ok` values with `||>` to chain dependent requests.
- Retry by passing a click signal to `refreshOn` in the source options.
