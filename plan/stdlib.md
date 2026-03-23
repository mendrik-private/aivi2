# Plan: AIVI standard library scope

## Status: design draft - not yet implemented

---

## 1. Overview

The legacy AIVI stdlib was broad: 64 modules and 1233 exported entries spanning
pure helpers, protocol clients, database layers, UI node DSLs, runtime
reactivity, and generic servers.

We should **not** port that library wholesale.

The new stdlib needs to follow the current AIVI model from `AGENTS.md` and
`AIVI_RFC.md`:

- pure user code by default
- one-shot effects through `Task E A`
- long-lived external input through `sig` plus `@source`
- strict closed-type decoding by default
- domain-backed wrappers such as `Duration`, `Url`, and `Path`
- GTK/libadwaita-first desktop programming on GNOME
- explicit runtime boundaries instead of ad hoc mutable handles

This document selects what to keep, what to redesign, what to defer, and what
not to port.

---

## 2. Selection rules

The stdlib should be chosen using the following rules.

1. Keep only surfaces that directly support the RFC's v1 language model.
2. Keep capabilities, not legacy APIs. Many old modules are worth keeping only
   after redesign.
3. Any legacy `Effect E A` API must become either:
   - `Task E A` for one-shot work, or
   - an `@source` provider for long-lived input.
4. Do not reimplement language or runtime features as library APIs:
   - no public mutable `Signal` API
   - no virtual DOM
   - no public scheduler model
   - no broad `Resource` choreography as the default user model
5. Prefer narrow, typed, GNOME-native integrations over generic cross-platform
   protocol stacks.
6. Prefer one canonical surface over duplicated facades such as `http`,
   `https`, `rest`, `net`, `number`, or `linalg`.
7. Keep pure foundation modules small and lawful.

---

## 3. Global migration rules from the legacy stdlib

### 3.1 Pure data and utility modules

Keep only the modules that fill clear gaps around the RFC's core types:
`List`, `Option`, `Result`, `Validation`, `Text`, defaults, and domain-backed
values such as `Duration`, `Url`, `Path`, and `Color`.

### 3.2 One-shot effects

Any old API that performed one request, one file write, one credential refresh,
or one process action should become a `Task`.

Examples:

- `http.fetch` -> `Task HttpError Response`
- `Goa.ensureCredentials` -> `Task GoaError Unit`
- `fs.write` -> `Task FsError Unit`

### 3.3 Long-lived or evented inputs

Anything that watches, subscribes, streams, polls, or listens should become a
source provider.

Examples from the RFC:

- `http.get`
- `fs.watch`
- `fs.read`
- `timer.every`
- `process.spawn`
- `mailbox.subscribe`
- window event sources

### 3.4 UI and scheduler concerns

UI description belongs in the language and GTK bridge, not in a library-level
widget tree API.

Scheduler, signal propagation, and subscription teardown are runtime
architecture concerns, not public stdlib data models.

---

## 4. Recommended retained surface

### 4.1 Pure foundation modules for v1

| Surface | Decision | Notes |
| --- | --- | --- |
| Root `aivi` / `aivi.prelude` | Keep, but shrink drastically | Export RFC core types and the small class surface the spec actually commits to. Do not keep a giant root namespace full of runtime handles. |
| `aivi.defaults` | Keep, redesign | Replace legacy `ToDefault` with RFC `Default`. The first required bundle is `Option`. |
| `aivi.list` | Keep | Small, lawful list helpers that complement `Functor`, `Applicative`, and `Monad`. |
| `aivi.option` | Keep | Small helper surface such as `isSome`, `isNone`, `getOrElse`, and conversion helpers. |
| `aivi.result` | Keep | Small helper surface such as `mapErr`, `toOption`, and `fromOption`. |
| `aivi.validation` | Keep, redesign | Align with RFC applicative accumulation and `Invalid (NonEmptyList E)`. |
| `aivi.text` | Keep, reduce | Keep Unicode-safe text and encoding helpers; avoid turning it into a catch-all parsing module. |
| `aivi.duration` | Keep, redesign as a domain | Match the RFC domain shape: explicit constructors, explicit `value`, and literal suffixes such as `ms`, `sec`, and `min`. |
| `aivi.url` | Keep, redesign as a domain | Match the RFC shape: `parse`, `value`, and focused helpers for query/parts. |
| `aivi.path` | Keep, redesign as a domain | Match the RFC shape: `parse`, `(/)`, `value`, and normalization behavior. |
| `aivi.color` | Keep, redesign as a domain | Keep a small GTK-friendly color domain instead of a large graphics utility module. |
| `NonEmpty` / `NonEmptyList` | Add | Needed by RFC `Validation`; not clearly present as a first-class legacy module. |

### 4.2 Runtime boundary modules we should keep

| Surface | Decision | Notes |
| --- | --- | --- |
| HTTP (`aivi.net.http` concept) | Keep, redesign | Keep request, response, header, and error concepts, but execution should be available as `Task` plus `@source http.*`. |
| File IO (`aivi.file` concept) | Keep, split | Replace handle-centric APIs with `fs.read` and `fs.watch` sources plus a small set of write/copy/delete tasks. |
| Timer (new provider) | Keep | The RFC explicitly recommends `timer.every` and `timer.after`. This should not be hidden inside a generic concurrency module. |
| Logging (`aivi.log`) | Keep, small | Keep structured logging and tracing tasks only. |
| Raw JSON (`aivi.json`) | Keep later, but as an escape hatch | Compiler-generated structural decoding is the default model. Raw `JsonValue` support is still useful, but it should not be the center of user-facing decoding. |

### 4.3 GNOME-first integrations we should keep

#### `aivi.gnome.onlineAccounts`

This capability is worth keeping because it matches the GNOME-first target
directly. The old API shape is not right.

What we should keep:

- account discovery
- provider identity
- account capability filtering
- attention-needed / unavailable states
- explicit credential refresh
- typed credential materialization where GOA supports it
- D-Bus-backed change observation

What we should **not** keep from the old API:

- mail-only modeling as the primary abstraction
- `imapConfig`
- `smtpConfig`
- `toImapConfig`
- `toSmtpConfig`

Those are app-domain adapters, not core stdlib responsibilities.

The replacement should be account-centric and source/task-shaped. Candidate
direction:

```aivi
type GoaAccountId
type GoaCapability
type GoaProvider
type GoaAccount = {
    id: GoaAccountId,
    provider: GoaProvider,
    label: Text,
    capabilities: Set GoaCapability,
    attentionNeeded: Bool
}

type GoaError = ...

@source goa.accounts with {
    capability: Mail
}
sig accounts : Signal (Result GoaError (List GoaAccount))

ensureCredentials : GoaAccountId -> Task GoaError Unit
accessToken : GoaAccountId -> Task GoaError AccessToken
```

Exact names can change, but the shape should remain:

- account model first
- D-Bus internal, typed boundary external
- one-shot credential work as `Task`
- account change observation as `@source`

#### PKCE loopback support

We do want PKCE support, but it should **not** reintroduce a general-purpose
HTTP server into the core stdlib.

The old `aivi.net.httpServer` is far too broad for the new language philosophy.
The retained capability should be a **narrow loopback-only auth helper**, not a
server framework.

Recommended replacement:

- add `aivi.auth.pkce` (or `aivi.oauth.pkce`)
- model a single auth flow
- generate verifier, challenge, and state explicitly
- bind only to localhost
- prefer ephemeral ports by default
- accept exactly one callback request
- surface typed success or typed failure
- shut the listener down deterministically after completion or cancellation

Candidate direction:

```aivi
type PkceSession
type PkceCallback = { code: Text, state: Text }
type PkceError = ...

begin : PkceConfig -> Task PkceError PkceSession
authorizeUrl : PkceSession -> Url
awaitCallback : PkceSession -> Task PkceError PkceCallback
cancel : PkceSession -> Task PkceError Unit
```

Important constraints:

- no arbitrary routing
- no middleware stack
- no websocket support
- no binding to public interfaces by default
- no general HTTP server lifecycle API in the first stdlib

The loopback listener exists only to complete a PKCE flow. Token exchange still
belongs on the normal HTTP surface.

---

## 5. Legacy module decision matrix

### 5.1 Keep, but redesign heavily

| Legacy module or family | Decision | Replacement direction |
| --- | --- | --- |
| `aivi` root builtins | Keep, shrink | Keep core types and constructors only; remove root runtime handles such as `httpServer`, `database`, `source`, and similar namespaces. |
| `aivi.prelude` | Keep, slim | Only re-export what the RFC explicitly wants in easy reach. |
| `aivi.defaults` | Keep | Match RFC `Default`, not legacy `ToDefault`. |
| `aivi.list` | Keep | Preserve a compact list helper layer. |
| `aivi.option` | Keep | Preserve compact option helpers. |
| `aivi.result` | Keep | Preserve compact result helpers. |
| `aivi.validation` | Keep | Redesign around `NonEmptyList` accumulation. |
| `aivi.text` | Keep | Smaller, clearer text surface. |
| `aivi.duration` | Keep | Domain over `Int`, not a `Span` record wrapper. |
| `aivi.url` | Keep | Domain over `Text`, explicit parse/value. |
| `aivi.path` | Keep | Domain over `Text`, explicit parse/value and path-join operator. |
| `aivi.color` | Keep | Domain-centered and GTK-oriented, not a general color toolkit. |
| `aivi.net.http` | Keep | Redesign around `Task` and `@source http.*`. |
| `aivi.file` | Keep conceptually | Split into `fs.read`, `fs.watch`, and focused write tasks. |
| `aivi.gnome.onlineAccounts` | Keep conceptually | Redesign around GOA accounts and credentials, not mail config bridging. |
| `aivi.log` | Keep | Minimal structured logging only. |

### 5.2 Defer until after the first stdlib wave

| Legacy module or family | Decision | Why it is deferred |
| --- | --- | --- |
| `aivi.json` | Defer, but keep small later | Useful as an interop escape hatch, but compiler-driven typed decoding should land first. |
| `aivi.regex` | Defer | Useful, but not a blocker for the GNOME-first core. |
| `aivi.i18n` | Defer and redesign | The old properties-style API is not obviously the right GNOME story; a gettext-oriented design is more likely. |
| `aivi.testing` | Defer | Important, but not required to define the runtime-facing stdlib scope. |
| `aivi.console` | Defer | More relevant for CLI tooling than for the flagship desktop app story. |
| `aivi.system` | Defer and limit | Environment/process access needs a tighter capability story. |
| `aivi.crypto` | Defer and narrow | Most legacy crypto should be internal or tightly scoped. Public crypto should appear only with a coherent bytes-first design. |
| `aivi.calendar` / `aivi.chronos.calendar` / `aivi.chronos.instant` / `aivi.chronos.timezone` | Defer | Dates and time zones are useful, but `Duration` is the urgent RFC-backed time surface. |
| `aivi.math` | Defer and curate | Do not port the full 74-entry module. Add only a clearly justified small numeric helper set later. |
| `aivi.number.bigint` / `aivi.number.decimal` | Defer as separate modules | `BigInt` and `Decimal` remain RFC core types, but separate facade modules are not a priority. |
| Future `process` and `mailbox` provider surfaces | Defer to phase 2 | RFC recommends them, but they should land after HTTP/fs/timer are stable. |
| Future GNOME secret-store support | Defer | If needed, it should be a GNOME/libsecret integration, not the old generic encrypted-blob API. |

### 5.3 Do not port to the new stdlib

| Legacy module or family | Decision | Why it should not return |
| --- | --- | --- |
| `aivi.reactive` | Do not port | `Signal` is now a language/runtime feature. We should not expose mutable `signal/get/set/watch` as the public model. |
| `aivi.ui` / `aivi.ui.gtk4` / `aivi.ui.layout` | Do not port | The RFC lowers markup directly to GTK/libadwaita. No virtual tree or raw widget-node DSL should be the public UI surface. |
| `aivi.ui.forms` | Do not port for v1 | Validation helpers can be built on `Validation`; they are not core enough to shape the stdlib. |
| `aivi.chronos.scheduler` | Do not port | The scheduler is runtime architecture, not a user-visible stdlib data model. |
| `aivi.net.httpServer` | Do not port as-is | Replace only the narrow PKCE loopback use case. |
| `aivi.net.https` | Do not port | TLS should be transport behavior of HTTP, not a separate module. |
| `aivi.rest` | Do not port | Redundant with a single canonical HTTP surface. |
| `aivi.net` facade | Do not port | Avoid umbrella namespace duplication. |
| `aivi.net.sockets` / `aivi.net.streams` | Do not port in the first stdlib | If revisited later, they should be source/task-shaped, not raw connection and stream APIs copied from the legacy design. |
| `aivi.concurrency` | Do not port | `Task`, runtime scheduling, and later mailbox sources replace raw `spawn`, `race`, and channel APIs. |
| `aivi.database` / `aivi.database.pool` | Do not port | Too large, too backend-oriented, and far from the GNOME-first v1 focus. |
| `aivi.email` | Do not port | GOA covers the immediate desktop integration need; IMAP/SMTP belongs in higher-level packages if it ever returns. |
| `aivi.secrets` | Do not port as-is | Generic encrypted-blob storage is the wrong abstraction. If needed later, use OS-backed credential storage. |
| `aivi.collections` plus `Queue` / `Deque` / `Heap` | Do not port | The RFC core collection story is `List`, `Map`, and `Set`. Extra collection towers can wait. |
| `aivi.logic` as a full class zoo | Do not port | Keep only the class surface the RFC explicitly commits to. |
| `aivi.number` / `aivi.linalg` facades | Do not port | Duplicate umbrella modules add noise without semantic value. |
| `aivi.number.complex` / `aivi.number.quaternion` / `aivi.number.rational` | Do not port | Not part of the current language focus. |
| `aivi.bits` / `aivi.generator` / `aivi.geometry` / `aivi.graph` / `aivi.linear_algebra` / `aivi.matrix` / `aivi.tree` / `aivi.units` / `aivi.vector` | Do not port | These are niche utility surfaces, not core to the current GNOME-first reactive language. |

---

## 6. New surfaces required by the current RFC, not by the old stdlib

The new stdlib should not just prune the old library. It also needs a few new
or newly explicit surfaces.

| Surface | Why it is needed |
| --- | --- |
| `Default` instance bundles | Required explicitly by RFC section 9. |
| `NonEmpty` / `NonEmptyList` | Required to make `Validation` match the RFC. |
| `DecodeMode` and related source option types | Needed for strict typed external decoding. |
| `timer` provider | Explicit RFC-recommended source family. |
| `fs.watch` and `fs.read` provider split | Explicit RFC recommendation; better than the old monolithic file API. |
| `goa` account change observation | Needed to make GNOME Online Accounts reactive instead of task-only. |
| `aivi.auth.pkce` | The legacy stdlib did not have the right narrow auth abstraction for PKCE loopback flows. |

---

## 7. Naming guidance

Avoid umbrella facades unless they provide real semantic value.

Recommended shape:

- pure modules: `aivi.list`, `aivi.option`, `aivi.result`, `aivi.validation`,
  `aivi.text`, `aivi.defaults`, `aivi.duration`, `aivi.url`, `aivi.path`,
  `aivi.color`
- provider namespaces for external input: `http`, `fs`, `timer`, `process`,
  `mailbox`, `window`, `goa`
- narrow integration modules: `aivi.gnome.onlineAccounts`,
  `aivi.auth.pkce`

Avoid reintroducing:

- `aivi.net`
- `aivi.rest`
- `aivi.https`
- `aivi.collections`
- `aivi.number`
- `aivi.linalg`

---

## 8. Suggested implementation order

### Phase 1: foundation and domains

- slim `aivi` / `aivi.prelude`
- `aivi.defaults`
- `aivi.list`
- `aivi.option`
- `aivi.result`
- `aivi.validation`
- `aivi.text`
- `aivi.duration`
- `aivi.url`
- `aivi.path`
- `aivi.color`
- `NonEmpty`

### Phase 2: core source and task surfaces

- HTTP request types plus `Task` and `@source http.*`
- `fs.read` and `fs.watch`
- timer providers
- minimal logging
- typed decode support and `DecodeMode`

### Phase 3: GNOME-first auth and account integration

- redesigned `aivi.gnome.onlineAccounts`
- PKCE loopback helper in `aivi.auth.pkce`
- any internal D-Bus/runtime plumbing needed for those surfaces

### Phase 4: later additions if real demand appears

- regex
- raw JSON escape hatch
- testing
- limited process and mailbox providers
- gettext-oriented i18n
- carefully scoped system and secret-store integrations

---

## 9. Final recommendation

The new AIVI stdlib should be **small, typed, and opinionated**.

Keep the pure foundation modules, keep HTTP/file/timer as first-class source and
task surfaces, keep GNOME Online Accounts as a real GNOME-native integration,
and add a narrow PKCE loopback helper.

Do **not** spend v1 effort porting the old generic server, reactive, database,
email, UI node, or utility-math ecosystems. Those belonged to the old language
shape, not the current one.
