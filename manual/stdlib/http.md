# aivi.http

Thin HTTP request helpers for text, bytes, status checks, and simple request bodies.

This module re-exports basic HTTP functions and adds a few convenience wrappers. In the current stdlib file, request URLs and errors are plain `Text` aliases.

## Import

```aivi
use aivi.http (
    Url
    HttpError
    ContentType
    StatusCode
    Header
    contentTypeJson
    contentTypeForm
    contentTypePlain
    contentTypeHtml
    get
    getBytes
    getStatus
    post
    put
    delete
    head
    postJson
    fetch
    fetchBytes
    checkStatus
    postJsonBody
    postForm
    putJsonBody
    remove
    fetchHeaders
)
```

## Type aliases and constants

```aivi
type HttpError = Text
type Url = Text
type ContentType = Text
type StatusCode = Int
type Header = (Text, Text)
```

- `HttpError` — task failure message text
- `Url` — a plain request URL string
- `ContentType` — the `Content-Type` header value sent with a body
- `StatusCode` — the numeric HTTP status
- `Header` — one response header as `(name, value)`

```aivi
contentTypeJson : ContentType
contentTypeForm : ContentType
contentTypePlain : ContentType
contentTypeHtml : ContentType
```

Ready-to-use content type constants for common request bodies.

## Overview

| Function | Type | Description |
|----------|------|-------------|
| `get` | `Url -> Task HttpError Text` | GET and return the response body as text |
| `getBytes` | `Url -> Task HttpError Bytes` | GET and return raw bytes |
| `getStatus` | `Url -> Task HttpError StatusCode` | GET and return only the status code |
| `post` | `Url -> ContentType -> Text -> Task HttpError Text` | POST a text body with an explicit content type |
| `put` | `Url -> ContentType -> Text -> Task HttpError Text` | PUT a text body with an explicit content type |
| `delete` | `Url -> Task HttpError Text` | DELETE and return the response body as text |
| `head` | `Url -> Task HttpError (List Header)` | HEAD request returning headers |
| `postJson` | `Url -> Text -> Task HttpError Text` | POST a JSON text body |
| `fetch` | `Url -> Task HttpError Text` | Short alias for `get` |
| `fetchBytes` | `Url -> Task HttpError Bytes` | Short alias for `getBytes` |
| `checkStatus` | `Url -> Task HttpError StatusCode` | Short alias for `getStatus` |
| `postJsonBody` | `Url -> Text -> Task HttpError Text` | Short alias for `postJson` |
| `postForm` | `Url -> Text -> Task HttpError Text` | POST using `contentTypeForm` |
| `putJsonBody` | `Url -> Text -> Task HttpError Text` | PUT using `contentTypeJson` |
| `remove` | `Url -> Task HttpError Text` | Short alias for `delete` |
| `fetchHeaders` | `Url -> Task HttpError (List Header)` | Short alias for `head` |

---

## Read requests

### `get` and `fetch`

```aivi
get : Url -> Task HttpError Text
fetch : Url -> Task HttpError Text
```

Both perform a GET request and return the response body as text. `fetch` is the shorter convenience name.

```aivi
use aivi.http (fetch)

type Url -> Task HttpError Text
func loadProfile = url =>
    fetch url
```

### `getBytes` and `fetchBytes`

```aivi
getBytes : Url -> Task HttpError Bytes
fetchBytes : Url -> Task HttpError Bytes
```

Use these when you need the raw response bytes instead of decoded text.

### `getStatus` and `checkStatus`

```aivi
getStatus : Url -> Task HttpError StatusCode
checkStatus : Url -> Task HttpError StatusCode
```

Use these when you only care whether an endpoint answered with the status you expect.

```aivi
use aivi.http (checkStatus)

type Url -> Task HttpError StatusCode
func checkHealth = url =>
    checkStatus url
```

### `head` and `fetchHeaders`

```aivi
head : Url -> Task HttpError (List Header)
fetchHeaders : Url -> Task HttpError (List Header)
```

Make a HEAD request and return the response headers without a body.

```aivi
use aivi.http (fetchHeaders)

type Url -> Task HttpError (List Header)
func inspectHeaders = url =>
    fetchHeaders url
```

---

## Write requests

### `post`

```aivi
post : Url -> ContentType -> Text -> Task HttpError Text
```

POST a text body and choose the `Content-Type` yourself.

```aivi
use aivi.http (
    post
    contentTypeForm
)

type Url -> Text -> Task HttpError Text
func submitLogin = endpoint formBody =>
    post endpoint contentTypeForm formBody
```

### `postJson` and `postJsonBody`

```aivi
postJson : Url -> Text -> Task HttpError Text
postJsonBody : Url -> Text -> Task HttpError Text
```

Send a JSON text body. `postJsonBody` is a friendlier name for the same shape.

```aivi
use aivi.http (postJsonBody)

type Url -> Text -> Task HttpError Text
func createTodo = endpoint jsonBody =>
    postJsonBody endpoint jsonBody
```

### `put` and `putJsonBody`

```aivi
put : Url -> ContentType -> Text -> Task HttpError Text
putJsonBody : Url -> Text -> Task HttpError Text
```

`put` lets you choose any content type. `putJsonBody` is the JSON shortcut.

### `delete` and `remove`

```aivi
delete : Url -> Task HttpError Text
remove : Url -> Task HttpError Text
```

Delete a resource and return the response body as text. `remove` is the shorter convenience name.
