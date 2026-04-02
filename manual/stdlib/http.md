# aivi.http

HTTP capability vocabulary plus the `HttpSource` handle type.

`aivi.http` no longer exports public request functions or a dedicated `HttpTask` alias. Use
`@source http` handles so signal-driven requests and on-demand request tasks share the same
capability boundary.

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
value healthCheck : Task Text Text = api.get "/health"
value healthStatus : Task Text Int = api.getStatus "/health"
```

## Exported vocabulary

- `HttpSource` - nominal handle annotation for `@source http`.
- `HttpError` - typed source-side failures: `Timeout`, `DecodeFailure`, `RequestFailure`.
- `HttpHeaders` / `HttpQuery` - request metadata maps.
- `HttpResponse A` - decoded signal result shape.
- `DecodeMode` and `Retry` - source option vocabulary.
- `contentType*`, `ContentType`, `StatusCode`, and `Header` - shared HTTP helper data.

Signal-backed request behavior and option support live in the [Built-in Source Catalog](/guide/source-catalog).
Direct handle-member request values such as `api.get "/health"` return ordinary `Task Text A`
values on the one-shot path.
