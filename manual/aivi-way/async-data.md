# Async Data

Async work should enter the program through `@source` signals or `Task` values, then get modeled explicitly with `Result`, `Option`, or a type of your own.

## Source-backed HTTP data

```aivi
use aivi.http (
    HttpError
    HttpResponse
    Timeout
    DecodeFailure
    RequestFailure
)

type DecodeMode =
  | Strict
  | Permissive

type User = {
    id: Int,
    name: Text
}

fun httpErrorText:Text error:HttpError =>
    error
     ||> Timeout               => "Request timed out"
     ||> DecodeFailure detail  => "Decode failed: {detail}"
     ||> RequestFailure detail => detail

fun responseText:Text response:(HttpResponse (List User)) =>
    response
     T|> "Loaded users"
     F|> httpErrorText

@source http.get "https://api.example.com/users" with {
    decode: Strict
}
sig users: Signal (HttpResponse (List User))
```

## Make loading explicit in your own type

```aivi
use aivi (
    Err
    Ok
)

use aivi.http (
    HttpError
    HttpResponse
    Timeout
    DecodeFailure
    RequestFailure
)

type LoadState A =
  | Loading
  | Loaded A
  | Failed Text

fun httpErrorText:Text error:HttpError =>
    error
     ||> Timeout               => "Request timed out"
     ||> DecodeFailure detail  => "Decode failed: {detail}"
     ||> RequestFailure detail => detail

fun rememberUsers:(LoadState (List Text)) response:(HttpResponse (List Text)) =>
    response
     ||> Ok items  => Loaded items
     ||> Err error => Failed (httpErrorText error)
```

AIVI does not give you a hidden loading flag. Model the states you care about.
