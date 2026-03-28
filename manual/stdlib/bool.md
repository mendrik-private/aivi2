# aivi.bool

Boolean utilities for AIVI. These functions complement the built-in `and`, `or`, and bool branching operators with named combinators for common logical patterns.

```aivi
use aivi.bool (not, xor, implies, both, either, neither, fromInt)
```

---

## not

Negates a boolean value.

```
not : Bool -> Bool
```

```aivi
use aivi.bool (not)

fun isInactive:Bool active:Bool =>
    not active
```

---

## xor

Returns `True` if exactly one of the two arguments is `True` (exclusive or).

```
xor : Bool -> Bool -> Bool
```

```aivi
use aivi.bool (xor)

fun toggleChanged:Bool previous:Bool current:Bool =>
    xor previous current
```

---

## implies

Logical implication: `implies a b` is `False` only when `a` is `True` and `b` is `False`.

```
implies : Bool -> Bool -> Bool
```

```aivi
use aivi.bool (implies)

fun checkRule:Bool hasPermission:Bool canAccess:Bool =>
    implies hasPermission canAccess
```

---

## both

Returns `True` if both arguments are `True`. Equivalent to `a and b`.

```
both : Bool -> Bool -> Bool
```

```aivi
use aivi.bool (both)

fun isAdminAndActive:Bool isAdmin:Bool isActive:Bool =>
    both isAdmin isActive
```

---

## either

Returns `True` if at least one argument is `True`. Equivalent to `a or b`.

```
either : Bool -> Bool -> Bool
```

```aivi
use aivi.bool (either)

fun canProceed:Bool hasTokenA:Bool hasTokenB:Bool =>
    either hasTokenA hasTokenB
```

---

## neither

Returns `True` only if both arguments are `False`.

```
neither : Bool -> Bool -> Bool
```

```aivi
use aivi.bool (neither)

fun isSilent:Bool isPlaying:Bool isPaused:Bool =>
    neither isPlaying isPaused
```

---

## fromInt

Converts an integer to a boolean: `0` becomes `False`, any other value becomes `True`.

```
fromInt : Int -> Bool
```

```aivi
use aivi.bool (fromInt)

fun hasFlags:Bool flagBits:Int =>
    fromInt flagBits
```
