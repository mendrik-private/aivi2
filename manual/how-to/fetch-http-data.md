# How to fetch HTTP data

Use an `http.get` source when you want external data to enter your app through a typed boundary and
flow into ordinary signals.

## Example

```aivi
type User = {
    id: Int,
    name: Text
}

@source http.get "https://api.example.com/users"
signal usersResult : Signal (Result HttpError (List User))

value main =
    <Window title="Users">
        <Box orientation="vertical" spacing={12} marginTop={16} marginBottom={16} marginStart={16} marginEnd={16}>
            <Button label="Refresh" onClick={usersResult.run} />
            <show when={usersResult.loading}>
                <Spinner />
            </show>
            <show when={usersResult.error}>
                <Label text="Loading failed" />
            </show>
            <match on={usersResult.success}>
                <case pattern={Some users}>
                    <Label text={"Users: {length users}"} />
                    <Box orientation="vertical" spacing={6}>
                        <each of={users} as={user} key={user.id}>
                            <Label text={user.name} />
                        </each>
                    </Box>
                </case>
            </match>
        </Box>
    </Window>

export main
```

## Why this shape works

1. `@source http.get ...` declares the outside-world boundary.
2. `usersResult.run` is a first-class retry signal, so the button stays a normal event hook.
3. `usersResult.loading`, `usersResult.success`, and `usersResult.error` expose the common request states directly.
4. The UI stays declarative: it branches on typed carriers instead of running a callback chain.

## Common variations

- Need stale successful data to survive a later failure? Fold the raw `Result` stream through
  [`aivi.async.AsyncTracker`](/stdlib/async).
- Need a typed client from an OpenAPI spec? Use [OpenAPI source guide](/guide/openapi-source).
- Need periodic refresh? Add a timer source or explicit `refreshOn` signal; `usersResult.run` still stays available.
