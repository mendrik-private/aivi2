# OpenAPI Source (`@source api`)

> Status: implemented — all layers from HIR elaboration through runtime worker.

## Overview

The `api` capability family provides a typed capability handle backed by an OpenAPI 3.x spec file.
It is structurally identical to the `http` family handle but adds:

- Compile-time operationId validation against the spec (when the spec path is a static literal).
- `baseUrl` option: the base URL is combined with the operation's path at runtime.
- `auth` option: accepts an `ApiAuth` sum value and injects the appropriate HTTP header.
- `aivi openapi-gen` CLI command: generates AIVI type declarations from spec schemas.

## Crate: `aivi-openapi`

Located at `crates/aivi-openapi/`. Modules:

| Module | Responsibility |
|--------|---------------|
| `model` | Serde-deserializable OpenAPI 3.x data model (OpenApiSpec, PathItem, Schema, …) |
| `parser` | Parse YAML or JSON spec file into `OpenApiSpec` |
| `resolver` | Resolve `$ref`s and derive `operationId`s; produces `ResolvedSpec` |
| `operations` | `OperationMethod`, `OperationInfo`, `find_operation`, `all_operations` |
| `auth` | `SecuritySchemeKind` derived from spec security schemes |
| `typegen` | `generate_aivi_types()` → AIVI source text with record, sum, and handle types |
| `diagnostics` | `SpecDiagnostic` with kind, message, and optional path |

Public entry point: `aivi_openapi::parse_spec_and_find_operation(path, operation_id)`.

## HIR layer (`aivi-hir`)

`capability_handle_elaboration.rs` handles the `Api` `BuiltinCapabilityFamily`:

- `lower_api_signal_member`: tries to extract the spec path as a plain text literal, calls
  `parse_spec_and_find_operation`, and either returns a `SourceDecorator` for `api.get` (GET ops)
  or emits a diagnostic (mutation ops, unknown ops). Falls back gracefully when the spec path is
  dynamic.
- `lower_api_value_member`: same spec lookup, returns an `HttpPost`/`HttpPut`/`HttpDelete`
  intrinsic for write operations.
- Both `supports_builtin_signal_member(Api, _)` and `supports_builtin_value_member(Api, _)` return
  `true` (all members are potentially valid; the actual distinction is spec-driven).

## Typing layer (`aivi-typing`)

Five new `BuiltinSourceProvider` variants in `source_contracts.rs`:
`ApiGet`, `ApiPost`, `ApiPut`, `ApiPatch`, `ApiDelete`.

All five share `api_options()` (HTTP options + `baseUrl: Text` + `auth: A`) with `HTTP_RECURRENCE`
and `HTTP_LIFECYCLE`.

## Runtime layer (`aivi-runtime`)

`providers.rs` adds:

- `ApiPlan::parse()`: reads `baseUrl` option, combines with operation path from argument 1,
  resolves auth option into HTTP headers, validates the final URL. All other options delegate to
  HTTP option parsers.
- `ApiPlan::into_http_plan()`: converts to `HttpPlan` (zero-copy hand-off).
- `spawn_api_worker()`: thin wrapper that calls `spawn_http_worker` with the converted plan.
- `run_http_request` updated to handle all five `Api*` provider variants in the `curl -X` argument.
- `extract_auth_header()`: maps `ApiAuth` sum variants to `Authorization` / `X-API-Key` headers.

## Stdlib: `aivi.api`

`stdlib/aivi/api.aivi` exports:

- `ApiError` sum with six constructors.
- `ApiAuth` sum: `BearerToken`, `BasicAuth`, `ApiKey`, `ApiKeyQuery`, `OAuth2`.
- `ApiSource = Unit` (the handle type).
- `ApiResponse A = (Result ApiError A)`.

## CLI: `aivi openapi-gen`

`crates/aivi-cli/src/main.rs` — `run_openapi_gen()`:

```
aivi openapi-gen <spec.yaml|spec.json> [-o output.aivi]
```

Parses → resolves → generates types. Outputs to file (`-o`) or stdout.

## Invariants

- The spec path is only used at compile time; it is not bundled or shipped.
- `baseUrl` is required at runtime; missing it causes `StartFailed`.
- Auth header injection is best-effort: unrecognised `ApiAuth` variants and `ApiKeyQuery` are
  silently skipped (no runtime error).
- All HTTP worker infrastructure (curl, cancellation, retry, backoff) is reused unchanged.

## See also

- [cli.md](cli.md) — `openapi-gen` command entry
- [runtime.md](runtime.md) — HTTP worker infrastructure
- `manual/guide/openapi-source.md` — user-facing guide
- `manual/guide/source-catalog.md` — full option reference
