# OpenAPI Sources

The `@source api` capability handle lets you connect to any HTTP API described by an OpenAPI 3.x
spec. The compiler validates operation names against the spec at compile time; the runtime executes
requests using the proven HTTP infrastructure.

## Declaring an API handle

```aivi
@source api "./petstore.yaml" with {
    baseUrl: serverUrl,
    auth: BearerToken apiToken,
    timeout: 30sec
}
signal petstore : ApiSource
```

- The first argument is the path to the OpenAPI spec file (YAML or JSON). It is resolved relative
  to the source file at compile time and is only used for validation — the runtime uses `baseUrl`.
- The `with { ... }` block accepts standard HTTP options (`timeout`, `retry`, `headers`,
  `refreshEvery`, `decode`, `refreshOn`, `activeWhen`) plus two API-specific options:

| Option | Type | Purpose |
| --- | --- | --- |
| `baseUrl` | `Text` | Required. Base URL prepended to operation paths at runtime. |
| `auth` | `ApiAuth` | Optional. Injects authentication headers from an `ApiAuth` sum value. |

## Using handle members

Member access on a handle is validated against the spec's `operationId`s at compile time.

### Read operations (GET) — use as `signal`

```aivi
signal allPets : Signal (Result ApiError (List Pet)) = petstore.listPets
```

`listPets` maps to `GET /pets` in the spec. The compiler checks that `listPets` is a real
operationId and that it is a GET/HEAD/OPTIONS operation. The signal is backed by `api.get` at
runtime.

### Write operations (POST / PUT / PATCH / DELETE) — use as `value`

```aivi
value createNewPet : (NewPet -> Task ApiError Pet) = petstore.createPet
```

`createPet` maps to `POST /pets` in the spec. The compiler lowers this to an `HttpPost` intrinsic
call with the composed URL.

## Generating type declarations

Use `aivi openapi-gen` to derive AIVI type declarations from the spec schemas:

```
aivi openapi-gen ./petstore.yaml -o types/petstore.aivi
```

The generated file contains:

- A record type for each component schema.
- A `type ApiError = ...` with standard error constructors.
- A `type ApiAuth = ...` with auth variants matching the spec's security schemes.
- A handle type alias (`type Petstore = Unit`).

You can then import the generated types in your module. The generated module path matches the output file you passed to `aivi openapi-gen` — for example, if you wrote the output to `types/petstore.aivi`, import from that module:

```aivi
use types.petstore (
    Pet
    NewPet
    ApiError
    ApiAuth
)
```

## Authentication

The `auth` option accepts an `ApiAuth` sum value imported from `aivi.api`:

| Variant | HTTP effect |
| --- | --- |
| `BearerToken Text` | `Authorization: Bearer <token>` header |
| `BasicAuth Text Text` | `Authorization: Basic <base64(user:pass)>` header |
| `ApiKey Text` | `X-API-Key: <key>` header |
| `ApiKeyQuery Text` | Key appended as query parameter (runtime: deferred) |
| `OAuth2 Text` | `Authorization: Bearer <token>` header |

## Full example

```aivi
use aivi.api (
    ApiSource
    ApiError
    BearerToken
)

type Pet = {
    id: Int,
    name: Text,
    status: Option Text
}

type NewPet = { name: Text }

value serverUrl : Text = "https://api.petstore.io/v2"
value apiToken : Text = "secret"

@source api "./petstore.yaml" with {
    baseUrl: serverUrl,
    auth: BearerToken apiToken
}
signal petstore : ApiSource

signal pets : Signal (Result ApiError (List Pet)) = petstore.listPets
```

## See also

- [Source Catalog](/guide/source-catalog) — full option reference for `api.get` and siblings
- [Sources](/guide/sources) — general sources tutorial
