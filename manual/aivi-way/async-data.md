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

```aivi
type HttpError = {
    message: Text,
    code: Int
}

type User = {
    id: Int,
    name: Text,
    email: Text,
    bio: Text
}

type LoadState A =
  | Loading
  | Loaded A
  | Failed Text

type Orientation =
  | Vertical
  | Horizontal

fun toLoadState:(LoadState User) #result:(Result HttpError User) =>
    result
     ||> Ok user => Loaded user
     ||> Err err => Failed err.message

fun nameFromState:Text #state:(LoadState User) =>
    state
     ||> Loading     => "Loading..."
     ||> Loaded user => user.name
     ||> Failed _    => "Unknown"

fun bioFromState:Text #state:(LoadState User) =>
    state
     ||> Loading     => ""
     ||> Loaded user => user.bio
     ||> Failed err  => err

@source http.get "/api/users/1"
sig userResponse : Signal (Result HttpError User)

sig userState : Signal (LoadState User) =
    userResponse
     |> toLoadState

sig userName : Signal Text =
    userState
     |> nameFromState

sig userBio : Signal Text =
    userState
     |> bioFromState

val main =
    <Window title="User Profile">
        <Box orientation={Vertical} spacing={8}>
            <Label text={userName} />
            <Label text={userBio} />
        </Box>
    </Window>

export main
```

## Handling the loading state

The above example maps `Loading` to a placeholder string. For a visible loading indicator:

```aivi
type HttpError = {
    message: Text,
    code: Int
}

type User = {
    id: Int,
    name: Text
}

type LoadState A =
  | Loading
  | Loaded A
  | Failed Text

fun toLoadState:(LoadState User) #result:(Result HttpError User) =>
    result
     ||> Ok user => Loaded user
     ||> Err err => Failed err.message

fun isLoadingState:Bool #state:(LoadState User) =>
    state
     ||> Loading  => True
     ||> Loaded _ => False
     ||> Failed _ => False

fun nameFromState:Text #state:(LoadState User) =>
    state
     ||> Loading     => ""
     ||> Loaded user => user.name
     ||> Failed _    => "Unknown"

type Orientation =
  | Vertical
  | Horizontal

@source http.get "/api/users/1"
sig userResponse : Signal (Result HttpError User)

sig userState : Signal (LoadState User) =
    userResponse
     |> toLoadState

sig isLoading : Signal Bool =
    userState
     |> isLoadingState

sig userName : Signal Text =
    userState
     |> nameFromState

val main =
    <Window title="Profile">
        <Box orientation={Vertical} spacing={8}>
            <show when={isLoading}>
                <Label text="Loading..." />
            </show>
            <Label text={userName} />
        </Box>
    </Window>

export main
```

## Retrying on error

```aivi
type HttpError = {
    message: Text,
    code: Int
}

type Payload = { data: Text }

provider button.clicked
    wakeup: sourceEvent
    argument id: Text

@source button.clicked "retry"
sig retryClicked : Signal Unit

@source http.get "/api/data" with {
    refreshOn: retryClicked
}
sig data : Signal (Result HttpError Payload)
```

Passing `refreshOn: retryClicked` tells the source to re-fetch when `retryClicked` fires.

## Chaining requests

When a second request depends on the result of a first, start by extracting the `Ok` value into
its own signal:

```aivi
type HttpError = {
    message: Text,
    code: Int
}

type User = {
    id: Int,
    name: Text
}

fun extractUserId:(Option Int) #result:(Result HttpError User) =>
    result
     ||> Ok user => Some user.id
     ||> Err _   => None

@source http.get "/api/users/1"
sig userResult : Signal (Result HttpError User)

sig userId : Signal (Option Int) =
    userResult
     |> extractUserId
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

```aivi
type HttpError = {
    message: Text,
    code: Int
}

type User = {
    id: Int,
    name: Text
}

fun extractUserId:(Option Int) #result:(Result HttpError User) =>
    result
     ||> Ok user => Some user.id
     ||> Err _   => None

@source http.get "/api/users/1"
sig userResult : Signal (Result HttpError User)

sig userId : Signal (Option Int) =
    userResult
     |> extractUserId
```

Each step is a separate named signal. No nesting, no error routing, no lifecycle cleanup.

## Summary

- `@source http.get "url"` produces a `Signal (Result HttpError T)`.
- Map the result through `||>` arms for `Ok` and `Err`.
- Use `LoadState A` or similar to represent `Loading` / `Loaded` / `Failed`.
- Extract `Ok` values into their own signals before wiring dependent work.
- Retry by passing a click signal to `refreshOn` in the source options.
