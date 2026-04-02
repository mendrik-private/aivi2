# aivi.http

HTTP capability vocabulary plus the `HttpSource` handle type.

`aivi.http` no longer exports public request functions such as `get`, `post`, or `delete`. Use
`@source http ...` so request/response signals and one-shot commands share the same provider
boundary.

## Import

```aivi
use aivi.http (
    HttpSource
    HttpError
    Timeout
    DecodeFailure
    RequestFailure
    HttpHeaders
    HttpQuery
    HttpResponse
    HttpTask
    DecodeMode
    Strict
    Permissive
    Retry
    ContentType
    StatusCode
    Header
    contentTypeJson
    contentTypeForm
    contentTypePlain
    contentTypeHtml
)
```

## Capability handle

```aivi
@source http "https://api.example.com"
signal api : HttpSource

signal users : Signal (HttpResponse (List User)) = api.get "/users"
value healthCheck : HttpTask Text = api.get "/health"
```

## Exported vocabulary

- `HttpError` — typed source-side failures: `Timeout`, `DecodeFailure`, `RequestFailure`.
- `HttpHeaders` / `HttpQuery` — request metadata maps.
- `HttpResponse A` — decoded signal result shape.
- `HttpTask A` — current one-shot task shape for direct `value = handle.member ...` uses.
- `DecodeMode` and `Retry` — source option vocabulary.
- `contentType*`, `ContentType`, `StatusCode`, and `Header` — shared HTTP helper data.

`HttpResponse A` uses structured `HttpError`. `HttpTask A` still uses the current `Task Text A`
command path because direct value-member lowering reuses the existing request intrinsics.
