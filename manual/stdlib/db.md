# aivi.db

Database records, query payloads, and task aliases.

This module is the data vocabulary for database-backed features. It describes connections, statements, parameters, paging options, and errors. The current stdlib file does not execute queries on its own.

## Import

```aivi
use aivi.db (
    DbError
    SchemaMismatch
    QueryFailed
    ConstraintViolation
    NestedTransaction
    ConnectionFailed
    SortDir
    Asc
    Desc
    Connection
    TableRef
    DbRow
    DbParam
    DbStatement
    DbPageOpts
    DbTask
)
```

## Overview

| Type | Purpose |
|------|---------|
| `Connection` | Where to connect |
| `TableRef A` | Named table reference with a change signal |
| `DbRow` | Raw row data keyed by column name |
| `DbParam` | One bound query parameter |
| `DbStatement` | SQL text plus bound parameters |
| `SortDir` | Sort direction |
| `DbPageOpts` | Limit/offset paging options |
| `DbError` | Structured database failures |
| `DbTask A` | Background database work returning `A` |

---

## `Connection`

```aivi
type Connection = {
    database: Text
}
```

A small record naming the database target to open. In practice this is often a filename or connection string.

```aivi
use aivi.db (Connection)

value appDb : Connection = {
    database: "data/app.db"
}
```

---

## `TableRef A`

```aivi
type TableRef A = {
    name: Text,
    conn: Connection,
    changed: Signal Unit
}
```

Reference to a table together with the connection it belongs to and a signal you can watch for refreshes. The type parameter `A` lets you label the kind of rows you expect to read from that table.

```aivi
use aivi.db (
    Connection
    TableRef
)

type User = {
    id: Int,
    email: Text
}

type Connection -> Signal Unit -> TableRef User
func usersTable = conn changed =>
    {
        name: "users",
        conn: conn,
        changed: changed
    }
```

---

## `DbRow`

```aivi
type DbRow = Dict Text Text
```

A raw result row keyed by column name. Every field value is stored as `Text`, so decoding into richer application types happens somewhere else.

```aivi
use aivi.db (DbRow)

value sampleRow : DbRow = {
    entries: [
        { key: "id", value: "7" },
        { key: "email", value: "ada@example.com" }
    ]
}
```

---

## `DbParam`

```aivi
type DbParam = {
    kind: Text,
    bool: Option Bool,
    int: Option Int,
    float: Option Float,
    decimal: Option Decimal,
    bigInt: Option BigInt,
    text: Option Text,
    bytes: Option Bytes
}
```

A bound query parameter. `kind` tells the database layer which field to read. The matching optional field carries the actual value.

```aivi
use aivi.db (DbParam)

type Text -> DbParam
func textParam = value =>
    {
        kind: "text",
        bool: None,
        int: None,
        float: None,
        decimal: None,
        bigInt: None,
        text: Some value,
        bytes: None
    }
```

---

## `DbStatement`

```aivi
type DbStatement = {
    sql: Text,
    arguments: List DbParam
}
```

A SQL statement paired with its bound arguments.

```aivi
use aivi.db (
    DbParam
    DbStatement
)

type Text -> DbParam
func emailParam = value =>
    {
        kind: "text",
        bool: None,
        int: None,
        float: None,
        decimal: None,
        bigInt: None,
        text: Some value,
        bytes: None
    }

type Text -> DbStatement
func findUserByEmail = email =>
    {
        sql: "select * from users where email = ?",
        arguments: [emailParam email]
    }
```

---

## `SortDir`

```aivi
type SortDir = Asc | Desc
```

Sort direction for APIs that let you choose ordering.

---

## `DbPageOpts`

```aivi
type DbPageOpts = {
    limit: Int,
    offset: Int
}
```

Simple paging options.

- `limit` — how many rows to ask for
- `offset` — how many rows to skip first

```aivi
use aivi.db (DbPageOpts)

value firstPage : DbPageOpts = {
    limit: 50,
    offset: 0
}
```

---

## `DbError`

```aivi
type DbError =
  | SchemaMismatch Text
  | QueryFailed Text
  | ConstraintViolation Text
  | NestedTransaction
  | ConnectionFailed Text
```

Structured failure reasons for database work.

- `SchemaMismatch Text` — the stored schema does not match what the code expects
- `QueryFailed Text` — the query could not be run
- `ConstraintViolation Text` — a constraint such as uniqueness or foreign keys was violated
- `NestedTransaction` — a second transaction was started before the first one finished
- `ConnectionFailed Text` — the database could not be opened or reached

```aivi
use aivi.db (
    DbError
    SchemaMismatch
    QueryFailed
    ConstraintViolation
    NestedTransaction
    ConnectionFailed
)

type DbError -> Text
func describeDbError = error => error
 ||> SchemaMismatch msg     -> "schema mismatch: {msg}"
 ||> QueryFailed msg        -> "query failed: {msg}"
 ||> ConstraintViolation msg -> "constraint violation: {msg}"
 ||> NestedTransaction      -> "nested transactions are not supported"
 ||> ConnectionFailed msg   -> "connection failed: {msg}"
```

---

## `DbTask`

```aivi
type DbTask A = Task DbError A
```

Alias for background database work that either returns `A` or fails with `DbError`.
