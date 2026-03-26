# Sources

`@source` attaches a runtime provider to a bodyless `sig`. The provider owns I/O, decoding, and wakeups; your program consumes the resulting typed signal.

## Built-in providers

```aivi
type HttpError =
  | Timeout
  | DecodeFailure Text

type User = {
    id: Int,
    name: Text
}

type FsWatchEvent =
  | Created
  | Changed
  | Deleted

type DecodeMode =
  | Strict
  | Permissive

type StreamMode =
  | Ignore
  | Lines
  | Bytes

domain Duration over Int
    literal s: Int -> Duration

domain Retry over Int
    literal x: Int -> Retry

sig apiHost = "https://api.example.com"

@source http.get "{apiHost}/users" with {
    decode: Strict,
    retry: 3x,
    timeout: 5s
}
sig users: Signal (Result HttpError (List User))

@source timer.every 120 with {
    immediate: True,
    coalesce: True
}
sig tick: Signal Unit

@source fs.watch "/tmp/demo.txt" with {
    events: [Created, Changed, Deleted]
}
sig fileEvents: Signal FsWatchEvent
```

## Provider options can depend on other declarations

```aivi
type HttpError =
  | Timeout
  | Cancelled

type Session = { token: Text }

type Credentials =
  | Credentials Text Text

type DecodeMode =
  | Strict
  | Permissive

type FsError = Missing | Denied

type FsWatchEvent =
  | Changed
  | Deleted

val loginBody = Credentials "demo" "secret"
val decodeMode = Strict

sig fileEvents: Signal FsWatchEvent = Changed

val reloadTrigger = fileEvents

@source http.post "/login" with {
    body: loginBody,
    decode: decodeMode
}
sig login: Signal (Result HttpError Session)

@source fs.read "/tmp/demo.txt" with {
    decode: decodeMode,
    reloadOn: reloadTrigger
}
sig fileText: Signal (Result FsError Text)
```

## Custom provider contracts

```aivi
type Mode = Stream | Snapshot

domain Duration over Int
    literal ms: Int -> Duration

provider custom.feed
    argument path: Text
    option timeout: Duration
    option mode: Mode
    wakeup: providerTrigger

@source custom.feed "/tmp/demo.txt" with {
    timeout: 5ms,
    mode: Stream
}
sig updates: Signal Int
```

Keep source docs conservative: if a provider family is not listed in `aivi.md` or exercised by the shipped fixtures, do not assume it exists.
