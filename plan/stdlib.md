# Plan: AIVI standard library scope

## Status: design draft — partially implemented (runtime, source, task layers active)

---

## 1. Goal

Define a small, typed, GNOME-first standard library that matches the current
language model in `AGENTS.md` and `AIVI_RFC.md`.

The anchor application is a native GNOME email client. It requires GNOME Online
Accounts, OAuth2 with PKCE, persistent local storage via SQL, and reactive inbox
state. The stdlib must be sufficient to build this application without reaching
outside its boundaries.

The stdlib centers five ideas:

- pure value-level programming by default
- one-shot effects through `Task E A`
- long-lived external input through `sig` plus `@source`
- explicit domain-backed wrappers for values such as `Duration`, `Url`, and `Path`
- local persistent state through `@source db.query` with compiler-lowered SQL

The initial stdlib should be opinionated and narrow. It should provide the
pieces needed for native desktop applications without turning the language into
a grab bag of unrelated utility packages.

---

## 2. Architectural rules

These rules are normative for the first stdlib wave.

1. Pure helpers stay pure. They must not hide runtime handles, blocking I/O, or
   mutable state.
2. One-shot external work uses `Task E A`.
3. Long-lived subscriptions, polling, watches, event feeds, and reactive database
   queries use `@source` providers.
4. Source options are closed and typed. Unknown or duplicate options are errors.
5. Source decoding is strict by default and uses typed error channels.
6. Public surfaces should expose domain values rather than raw carrier types
   when invariants matter.
7. GTK, D-Bus, network clients, file watching, database access, and similar
   runtime integrations remain behind controlled effect or source boundaries.
8. The stdlib must not re-expose runtime internals as public APIs:
   - no public mutable `Signal` API
   - no public scheduler API
   - no generic UI tree API
   - no general resource choreography as the default user model
9. Prefer one canonical surface per capability. Avoid duplicated umbrella
   namespaces.
10. Database sources follow the same lifecycle contract as HTTP sources:
    reactive reconfiguration is transactional, stale publications are suppressed,
    and mutation is Task-only.

---

## 3. First-wave modules to implement

### 3.1 Core export surface

#### `aivi`

Keep the root surface small.

It should export only:

- core types committed to by the RFC
- core constructors such as `Some`, `None`, `Ok`, `Err`, `Valid`, `Invalid`
- the small class surface that the language commits to

It should **not** become a namespace full of runtime handles or subsystem
facades.

#### `aivi.prelude`

Provide a compact import surface for ordinary programs.

The prelude should re-export:

- primitive types
- `List`, `Option`, `Result`, `Validation`, `Signal`, `Task`
- the core class surface actually required by the RFC
- a minimal set of high-value helpers

The prelude should stay intentionally small.

### 3.2 Pure foundation modules

These modules should be part of the first implementation wave.

| Module | What to implement | Shape notes |
| --- | --- | --- |
| `aivi.defaults` | `Default` instance bundles | The first required bundle is `Option`. Record omission support should rely on this module. |
| `aivi.list` | compact list helper set | Focus on traversal, search, partitioning, zipping, and safe access. |
| `aivi.option` | compact option helper set | `isSome`, `isNone`, `getOrElse`, and conversion helpers. |
| `aivi.result` | compact result helper set | `isOk`, `isErr`, `mapErr`, and conversion helpers. |
| `aivi.validation` | applicative validation surface | Match RFC accumulation semantics with `NonEmptyList`. |
| `aivi.text` | Unicode-safe text and encoding helpers | Keep it focused on text operations, encoding, and parsing helpers that clearly belong here. |
| `aivi.duration` | domain-backed duration type | Explicit constructors, explicit `value`, literal suffixes, and domain-local operators. |
| `aivi.url` | domain-backed URL type | Explicit parse and explicit unwrap. |
| `aivi.path` | domain-backed path type | Explicit parse, explicit unwrap, and path-join operator. |
| `aivi.color` | domain-backed color type | Keep it small and GTK-friendly. |
| `aivi.nonEmpty` | `NonEmpty` / `NonEmptyList` | Needed to make `Validation` match the RFC cleanly. |

### 3.3 Domain shapes

The domain modules should follow the RFC's explicit-construction model.

#### `aivi.duration`

Recommended surface:

```aivi
domain Duration over Int
    literal ms  : Int -> Duration
    literal sec : Int -> Duration
    literal min : Int -> Duration
    millis      : Int -> Duration
    trySeconds  : Int -> Result DurationError Duration
    value       : Duration -> Int
    (+)         : Duration -> Duration -> Duration
    (-)         : Duration -> Duration -> Duration
```

#### `aivi.url`

Recommended surface:

```aivi
domain Url over Text
    parse : Text -> Result UrlError Url
    value : Url -> Text
```

Add only focused helpers that preserve the explicit domain model.

#### `aivi.path`

Recommended surface:

```aivi
domain Path over Text
    parse : Text -> Result PathError Path
    (/)   : Path -> Text -> Path
    value : Path -> Text
```

Path normalization should be part of the domain's invariant story, not a loose
string helper.

#### `aivi.color`

Use a domain-backed color representation with a small constructor and unwrap
surface. The goal is to support GTK-facing style and property work, not to ship
an extensive graphics toolkit.

---

## 4. Runtime boundary surfaces

### 4.1 HTTP

Implement HTTP as:

- a typed request/response surface
- one-shot `Task` entry points for imperative use
- an `@source` provider family for reactive use

Required user-facing source surface:

```aivi
@source http.get "/users"
sig users : Signal (Result HttpError (List User))

@source http.post "/login" with {
    body: creds,
    headers: authHeaders,
    decode: Strict,
    timeout: 5sec
}
sig login : Signal (Result HttpError Session)
```

Required option concepts:

- `headers`
- `query`
- `body`
- `decode`
- `timeout`
- `retry`
- `refreshOn`
- `refreshEvery`
- `activeWhen`

Required runtime behavior:

- request-like sources must cancel in-flight work or mark stale results so they
  cannot publish into the live graph
- reconfiguration must be transactional
- decoding happens before publication
- failures stay typed

### 4.2 Filesystem

Implement filesystem support as two distinct source families plus a small task
surface.

Required source surface:

```aivi
@source fs.watch configPath with {
    events: [Created, Changed, Deleted]
}
sig fileEvents : Signal FsEvent

@source fs.read configPath with {
    decode: Strict,
    reloadOn: fileEvents
}
sig fileText : Signal (Result FsError Text)
```

Required rules:

- `fs.watch` publishes events only
- `fs.read` publishes snapshots only
- reads and watches are separate concepts
- file path inputs should use the `Path` domain where practical

The task surface should stay small and explicit:

- write text or bytes
- delete
- create directories if needed
- optionally rename or copy if clearly justified

### 4.3 Timer

Implement a dedicated `timer` provider family.

Required surface:

```aivi
@source timer.every 120ms
sig tick : Signal Unit

@source timer.after 1sec
sig ready : Signal Unit
```

Required option concepts:

- `immediate`
- `jitter`
- `coalesce`
- `activeWhen`

### 4.4 Logging

Implement a minimal structured logging surface under `aivi.log`.

It should support:

- a closed log-level enum
- message text
- structured key-value context
- one-shot logging tasks

This surface is for tracing, diagnostics, and application logs. It should stay
small and not grow into a general observability framework.

### 4.5 Database

Implement local persistent storage as:

- a typed reactive query surface via `@source db.query`
- one-shot mutation `Task` entry points
- a transaction combinator for atomic multi-step mutations

The database provider targets a local embedded SQL engine (SQLite via a
GNOME-friendly binding). It is not a general RDBMS abstraction layer. The
surface is intentionally narrow: it covers what a local-first desktop
application needs, not a server-side ORM.

#### Query source model

Reactive queries follow the same lifecycle contract as HTTP sources:

- the source activates when the signal is first observed
- reactive inputs in `with {}` trigger transactional reconfiguration
- stale publications from superseded query generations are suppressed
- `activeWhen` suspends the query without tearing down the schema connection

Required source surface:

```aivi
@source db.query Email with {
    where: { folder: currentFolder, isDeleted: False },
    orderBy: [{ field: .receivedAt, dir: Desc }],
    limit: pageSize
}
sig emails : Signal (Result DbError (List Email))

@source db.query Thread with {
    where: { accountId: account.id },
    include: [.messages, .labels],
    orderBy: [{ field: .lastMessageAt, dir: Desc }]
}
sig threads : Signal (Result DbError (List Thread))
```

Required option concepts:

- `where`: a typed field-predicate record matched against the row type `T`
- `orderBy`: a list of `{ field, dir }` records; `dir` is `Asc` or `Desc`
- `limit`: a positive integer or reactive `Signal Int`
- `offset`: a non-negative integer or reactive `Signal Int` for pagination
- `include`: a list of relation paths to eagerly fetch via JOIN
- `refreshOn`: a `Signal B` that forces re-execution on update
- `activeWhen`: a `Signal Bool` gate

#### SQL lowering

The `db.query` provider lowers the `with {}` option record to SQL at compile
time. This is the mechanism by which writing AIVI query options produces SQL
without user code touching a query builder directly.

The lowering rules are:

- `where` record fields map to `WHERE col = ?` clauses joined by `AND`; nested
  field access maps to joined relation columns
- `orderBy` maps to `ORDER BY col ASC|DESC`
- `limit` and `offset` map to `LIMIT ? OFFSET ?` with bound parameters
- `include` paths trigger `LEFT JOIN` clauses for declared relation fields

Reactive expressions in any option position become bound SQL parameters that
are substituted at query execution time, not embedded as literals. When a
reactive input changes, the runtime re-executes the compiled query plan with the
new parameter values rather than regenerating SQL.

The row type `T` is the source of truth for the schema. The compiler validates
option field references against `T`'s declared fields at elaboration time. An
unknown field in `where`, an unknown relation in `include`, or a direction value
other than `Asc`/`Desc` is a compile-time error.

#### Schema and table binding

A schema record type is an ordinary AIVI `type` declaration. The database
provider infers the SQL table name from the type name by convention (snake_case
of the type name). An explicit table annotation is not needed for v1;
convention-based resolution keeps the surface minimal.

Relation fields are declared as list-typed or option-typed fields within the
record and are resolved against foreign-key constraints declared in the
migration files. The compiler validates option field references against the
declared AIVI record at elaboration time. At startup the runtime verifies that
the applied migration state matches the schema version the app was compiled
against; a mismatch raises `DbError.SchemaMismatch` before any query executes.

#### Migrations

The AIVI record types are the schema source of truth. Migration management is
handled by two CLI commands rather than an external tool.

`aivi db migrate` compares the current AIVI record types in the workspace
against the last applied migration state and generates a new SQL migration file
in `db/migrations/`. The developer reviews and commits the generated file before
deploying. No SQL is auto-applied without review.

`aivi db apply` applies any pending migration files in `db/migrations/` in
lexicographic order using a `_schema_migrations` tracking table. It is
idempotent and safe to run at application startup or from the CLI.

Required behavior:

- generated migration files are plain SQL; no AIVI-specific syntax
- each migration file has a timestamp prefix for stable ordering
- the `_schema_migrations` table records which files have been applied
- `aivi db migrate` errors if the workspace has uncommitted migration files that
  have not yet been applied, preventing schema drift
- `aivi db apply` runs inside a single transaction; a failed migration rolls
  back completely and does not advance the applied state

#### Mutation task surface

Mutations are one-shot tasks. They do not produce reactive updates directly.
Reactive query sources that cover the mutated rows will pick up the change on
the next scheduled query execution or via the `refreshOn` mechanism.

Required mutation surface:

```aivi
db.insert : T -> Task DbError T
db.update : T -> Task DbError T
db.delete : T -> Task DbError Unit
db.upsert : T -> Task DbError T
```

The insert and upsert variants return the persisted row including any
database-generated fields (auto-incremented IDs, server-side timestamps).

#### Transaction combinator

Atomic multi-step mutations use a `Task`-level transaction combinator:

```aivi
db.transaction : Task DbError A -> Task DbError A
```

All mutation tasks inside a `db.transaction` body either commit atomically or
roll back as a unit. Transactions do not nest in v1; a `db.transaction` inside
another `db.transaction` raises a `DbError.NestedTransaction` at runtime.

Transactions are not woven through reactive query sources. A committed
transaction does not directly push a publication into the scheduler. Reactive
queries that cover affected rows will pick up the change on their next
configured wakeup (next `refreshOn` trigger or next timer tick).

### 4.6 Auth: OAuth2 with PKCE

Implement an auth provider surface under `aivi.auth` that covers the OAuth2
PKCE flow required for non-GOA providers and for providers where GOA does not
supply the needed credential type.

PKCE is required for the email client use case: it is the standard mechanism
for obtaining OAuth2 tokens from providers (such as Gmail and Outlook) in
contexts where a client secret cannot be embedded securely.

This section covers the PKCE flow specifically. GOA-backed credentials are
handled separately in §5.1.

Required surface:

```aivi
type PkceConfig = {
    clientId     : Text,
    authEndpoint : Url,
    tokenEndpoint: Url,
    scopes       : List Text,
    redirectPort : Int
}

type PkceToken = {
    accessToken  : Text,
    refreshToken : Option Text,
    expiresAt    : Option Int
}

type PkceError
    = UserCancelled
    | NetworkError Text
    | InvalidResponse Text
    | Timeout

auth.pkce.authorize : PkceConfig -> Task PkceError PkceToken
auth.pkce.refresh   : PkceConfig -> Text -> Task PkceError PkceToken
```

Required runtime behavior:

- `auth.pkce.authorize` opens a temporary localhost HTTP listener on
  `redirectPort`, launches the system browser to the authorization URL with a
  PKCE challenge, waits for the redirect callback, exchanges the code for
  tokens, and closes the listener
- the listener must shut down whether the flow succeeds, fails, or times out;
  no dangling ports
- the code verifier is generated internally and never exposed to user code
- `auth.pkce.refresh` exchanges a refresh token for a new access token without
  browser interaction
- token storage is the user's responsibility; the runtime does not persist
  tokens automatically

This surface is intentionally minimal. It provides the PKCE flow as a `Task`
and leaves token storage, expiry tracking, and proactive refresh scheduling to
the application layer. A higher-level credential manager can be built on top in
a later phase if demand emerges.

### 4.7 Mail protocols: IMAP and SMTP

For the GNOME email client, IMAP and SMTP are first-wave requirements. The
architecture is local-first: IMAP drives a background sync process that writes
into the local SQLite database; the UI reads from the database via `db.query`.
SMTP sends are one-shot tasks.

This separation keeps the UI reactive over the local database rather than
directly over a live IMAP connection, which simplifies offline behavior, search,
and threading.

#### IMAP sync source

IMAP sync is a long-lived background process. It holds an IMAP IDLE connection
per folder and writes new or updated messages into the local database.

Required source surface:

```aivi
@source imap.sync account with {
    credentials: tokenForAccount,
    folders: [Inbox, Sent, Drafts, Trash],
    pollEvery: 30sec,
    activeWhen: isOnline
}
sig syncState : Signal (Result ImapError SyncState)
```

`syncState` carries a typed summary of the sync process (last synced timestamp,
in-progress flag, error if any). The actual message data lands in the local
database and is read via `db.query`, not via this signal.

Required option concepts:

- `credentials`: an `AccessToken` from GOA or a `PkceToken.accessToken`
- `folders`: the list of folder names to sync
- `pollEvery`: fallback polling interval when IMAP IDLE is unavailable
- `activeWhen`: suspend sync when offline or account is not active

Required types:

```aivi
type SyncState = {
    lastSyncedAt : Option Int,
    inProgress   : Bool,
    error        : Option ImapError
}

type ImapError
    = AuthFailed
    | ConnectionFailed Text
    | FolderNotFound Text
    | ProtocolError Text
```

Required runtime behavior:

- the provider maintains one IMAP IDLE connection per configured folder
- on `map` / new message notification, fetches message headers and bodies,
  writes to local DB via the `aivi.db` layer, then publishes an updated
  `SyncState`
- when `activeWhen` becomes `False`, the IDLE connections are closed cleanly;
  when it becomes `True`, sync resumes from the last known UID watermark
- credential errors surface through `SyncState.error`, not through unhandled
  exceptions; the source does not tear down on a single auth failure but
  publishes the error and waits for credential refresh

#### SMTP task surface

Email sending is a one-shot task.

Required surface:

```aivi
type SmtpConfig = {
    host        : Text,
    port        : Int,
    credentials : AccessToken
}

type SmtpMessage = {
    from        : Text,
    to          : List Text,
    cc          : List Text,
    bcc         : List Text,
    subject     : Text,
    bodyText    : Text,
    bodyHtml    : Option Text,
    attachments : List Attachment
}

type SmtpError
    = AuthFailed
    | ConnectionFailed Text
    | RecipientRejected Text
    | MessageTooLarge
    | ProtocolError Text

smtp.send : SmtpConfig -> SmtpMessage -> Task SmtpError Unit
```

For GOA-backed accounts the `SmtpConfig` can be derived from the GOA account
metadata; no manual host/port configuration is required in that case.

---

## 5. GNOME-first integration surfaces

### 5.1 `aivi.gnome.onlineAccounts`

This module should provide a typed GNOME Online Accounts boundary.

Its design should be account-centric, not protocol-centric.

Required concepts:

- account identity
- provider identity
- capability filtering
- attention-needed state
- typed account listing
- typed credential refresh
- typed token retrieval where GOA supports it
- account change observation

Recommended shape:

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
accessToken       : GoaAccountId -> Task GoaError AccessToken
oauthToken        : GoaAccountId -> Task GoaError OAuthToken
```

Implementation guidance:

- use D-Bus internally
- keep D-Bus details out of the language-facing types
- expose only typed account and credential concepts
- publish account changes through a source, not polling hidden inside helpers
- `oauthToken` should return a typed record containing the access token, token
  type, and expiry hint, not a raw string
- when GOA signals that a credential needs attention, surface that through the
  `attentionNeeded` field on the account rather than raising an error at the
  call site; errors from `ensureCredentials` indicate actual failure, not
  user-interaction requirements

#### Credential handoff for the email client

The email client needs to route GOA credentials into HTTP requests. The
recommended handoff pattern:

```aivi
@source goa.accounts with { capability: Mail }
sig mailAccounts : Signal (Result GoaError (List GoaAccount))

sig accessTokenForAccount =
    selectedAccount
     |> .id
     |> ensureCredentials
```

`ensureCredentials` returns `Task GoaError Unit`; the result is used to gate
HTTP sources via `activeWhen` or `refreshOn` rather than being threaded through
a runtime handle.

### 5.2 GTK markup: `trackVisible`

`trackVisible={signal}` is a GTK markup attribute that routes widget visibility
lifecycle events into a user-declared `Signal Bool` input signal.

```aivi
sig inboxVisible : Signal Bool

@source db.query Email with {
    where: { folder: currentFolder },
    activeWhen: inboxVisible
}
sig emails : Signal (Result DbError (List Email))

<Stack>
    <InboxView trackVisible={inboxVisible} emails=emails />
    <SettingsView />
</Stack>
```

When `InboxView` is mapped to screen, the runtime publishes `True` into
`inboxVisible`. When it is unmapped (hidden, replaced by another view, or
destroyed), it publishes `False`. The `db.query` source suspends when
`inboxVisible` is `False` and resumes with a fresh query execution when it
becomes `True` again.

Required behavior:

- the GTK host publishes `False` immediately at signal registration, before the
  first map event, so the query never activates before the widget is on screen
- subsequent `map` / `unmap` events publish `True` / `False` respectively
- `map` / `unmap` is used rather than `show` / `hide` because a widget can be
  shown but not yet mapped (e.g. inside an unshown parent container)
- the bound signal must be a body-less annotated `Signal Bool` input signal;
  binding a derived signal is a compile-time error

The visibility signal is a plain `Signal Bool`. It is not coupled to
`activeWhen` automatically; the user wires it explicitly. This keeps the two
concerns separate: `trackVisible` tracks screen presence, `activeWhen` controls
source activation. A visibility signal can also be used for lazy image loading,
animation triggers, analytics, or any other purpose.

Queries that live outside the UI (background sync, scheduled polling) do not
need `trackVisible` at all. `activeWhen` accepts any `Signal Bool` regardless of
how it is produced.

---

## 6. What is not in the first stdlib wave

The first wave should stay focused. The following areas are out of scope unless
later work proves they are necessary:

- generic secret-storage APIs (use GOA or the PKCE task surface instead)
- raw sockets and generic streaming APIs
- general HTTP server frameworks
- public signal or scheduler manipulation APIs
- UI tree or form helper DSLs
- broad math, graph, geometry, matrix, vector, or linear-algebra libraries
- large generic crypto toolkits
- generic RDBMS abstraction layers or ORM query planners beyond the `db.query`
  surface defined in §4.5

These capabilities can be reconsidered later, but they should not shape the v1
stdlib architecture.

---

## 7. Later phases

These are reasonable follow-on candidates after the first wave is stable:

- raw JSON escape hatch APIs
- regex
- testing helpers
- gettext-oriented i18n
- limited process and mailbox provider surfaces
- carefully scoped system access
- GNOME-native secret-store integration if real needs appear
- calendar and time-zone support once the domain and source foundations are solid
- SQL predicate pushdown from pipe algebra: once the `db.query` source is
  stable, the compiler can analyze pipe chains that follow a `db.query` source
  and push eligible `?|>` and `|>` stages down into the SQL plan rather than
  executing them in-process; this is an optimization, not a correctness
  requirement
- higher-level credential manager built on the PKCE task surface
- calendar and contact sync over CardDAV / CalDAV once IMAP sync is stable

Later work should reuse the same rules:

- pure helpers stay pure
- one-shot work uses `Task`
- long-lived input uses `@source`
- no duplicate facades

---

## 8. Implementation order

### Phase 1: core foundation

- `aivi`
- `aivi.prelude`
- `aivi.defaults`
- `aivi.list`
- `aivi.option`
- `aivi.result`
- `aivi.validation`
- `aivi.nonEmpty`
- `aivi.text`
- `aivi.duration`
- `aivi.url`
- `aivi.path`
- `aivi.color`

### Phase 2: source and task boundaries

- HTTP types plus `http` provider family
- filesystem types plus `fs.read` and `fs.watch`
- timer provider family
- minimal `aivi.log`
- typed decode support and source option types

### Phase 3: GNOME-native account support and auth

- `aivi.gnome.onlineAccounts`
- internal D-Bus plumbing needed for that surface
- `aivi.auth` PKCE task surface (required alongside GOA for providers that
  need a manual OAuth2 flow)

### Phase 4: database

- `aivi.db` with `db.query` source family and mutation task surface
- SQL lowering of `where`, `orderBy`, `limit`, `offset`, and `include` options
- `db.transaction` combinator
- schema validation against declared AIVI record types
- `aivi db migrate` and `aivi db apply` CLI commands
- startup schema version check

### Phase 5: mail protocols

- `imap.sync` source provider with IDLE connection management
- `smtp.send` task
- credential handoff from GOA and PKCE token surfaces
- typed `ImapError` and `SmtpError`

### Phase 6: later expansions

- JSON escape hatch
- regex
- testing
- i18n
- process and mailbox providers
- SQL predicate and projection pushdown from pipe algebra chains
- limited system and secret-store integrations
- CardDAV / CalDAV sync once mail protocols are stable

---

## 9. Definition of done

This plan is complete only when the implementation follows these constraints:

1. The first stdlib wave is small and coherent.
2. Public APIs clearly separate pure helpers, `Task` work, and `@source`
   providers.
3. Domain-backed values enforce explicit construction and explicit unwrapping.
4. No umbrella duplicate namespaces are introduced.
5. No public API re-exposes signal mutation, scheduler control, or UI tree
   machinery.
6. GOA support matches the GNOME-first philosophy and remains typed, narrow, and
   deterministic.
7. The database source follows the same lifecycle contract as the HTTP source:
   reactive reconfiguration is transactional, stale publications are suppressed,
   and the `activeWhen` gate works correctly.
8. The PKCE auth surface opens and closes its localhost listener cleanly under
   success, failure, and timeout conditions.
9. `aivi db migrate` generates valid SQL for all supported field type changes;
   `aivi db apply` is idempotent and rolls back atomically on failure.
10. The `imap.sync` source closes IDLE connections cleanly when `activeWhen`
    becomes `False` or the app shuts down.
11. Tests cover:
    - domain invariants
    - strict decode behavior
    - source reconfiguration and stale-result suppression
    - GOA account change delivery
    - `db.query` reconfiguration when reactive `where` inputs change
    - `db.transaction` rollback on task failure
    - PKCE listener teardown under all exit conditions
    - migration apply idempotency and rollback
    - IMAP IDLE reconnect after transient connection failure
    - SMTP credential error surfaced as typed `SmtpError`

---

## 10. Final recommendation

Implement the smallest stdlib that makes the GNOME email client real:

- a strong pure foundation
- explicit domains
- source-first external input
- task-based one-shot effects
- GNOME Online Accounts with typed credential handoff
- OAuth2 PKCE for providers outside GOA
- local persistent storage via reactive SQL-lowered `db.query` sources and
  mutation tasks
- `aivi db migrate` / `aivi db apply` for schema management
- IMAP sync source and SMTP send task for mail protocol access

Everything else should wait until it is justified by the current architecture.
