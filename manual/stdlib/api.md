# aivi.api

Shared auth and error vocabulary for the `@source api` / OpenAPI capability surface.

This module is intentionally small: it does not issue requests on its own. It provides the sum and
alias types that the OpenAPI source guide and runtime use when configuring API-backed signals and
tasks.

```aivi
use aivi.api (
    ApiAuth
    ApiError
    ApiResponse
    BearerToken
)
```

---

## `ApiError`

The current API error surface:

```aivi
type ApiError =
  | ApiTimeout
  | ApiDecodeFailure Text
  | ApiRequestFailure Text
  | ApiUnauthorized
  | ApiNotFound
  | ApiServerError Text
```

```aivi
use aivi.api (
    ApiDecodeFailure
    ApiError
    ApiNotFound
    ApiRequestFailure
    ApiServerError
    ApiTimeout
    ApiUnauthorized
)

type ApiError -> Text
func describeApiError = error => error
 ||> ApiTimeout                -> "Timed out"
 ||> ApiDecodeFailure message  -> "Decode failed: " + message
 ||> ApiRequestFailure message -> "Request failed: " + message
 ||> ApiUnauthorized           -> "Unauthorized"
 ||> ApiNotFound               -> "Not found"
 ||> ApiServerError message    -> "Server error: " + message
```

---

## `ApiAuth`

Auth configuration for API-backed sources and operations:

```aivi
type ApiAuth =
  | BearerToken Text
  | BasicAuth Text Text
  | ApiKey Text
  | ApiKeyQuery Text
  | OAuth2 Text
```

```aivi
use aivi.api (
    ApiAuth
    BearerToken
)

value auth : ApiAuth = BearerToken "secret-token"
```

`ApiKeyQuery` is the query-parameter variant. `BearerToken` and `OAuth2` both map to bearer-style
authorization headers in the current runtime.

---

## `ApiSource`

Marker type used by the OpenAPI capability layer.

```aivi
use aivi.api (ApiSource)

type SourceHandle = ApiSource
```

---

## `ApiResponse A`

Convenience alias for the current API result shape:

```aivi
use aivi.api (ApiError)

type ApiResponse A =
  Result ApiError A
```

```aivi
use aivi.api (ApiResponse)

type User = {
    id: Int,
    name: Text
}

value userResult : ApiResponse User =
    Ok {
        id: 1,
        name: "Ada"
    }
```
