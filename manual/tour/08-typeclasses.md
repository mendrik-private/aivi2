# Type Classes

AIVI supports explicit `class` and `instance` declarations. Keep the mental model narrow: declare members on the class, then implement them for a concrete type.

## Declaring a class and instance

```aivi
class Eq A
    (==): A -> A -> Bool

type Blob = Blob Bytes

fun blobEquals:Bool left:Blob right:Blob =>
    True

instance Eq Blob
    (==) left right = blobEquals left right

fun sameBlob:Bool left:Blob right:Blob =>
    left == right
```

## Another class shape

```aivi
class Compare A
    same: A -> A -> Bool

type Label = Label Text

instance Compare Label
    same left right = left == right
```

The bundled environment also exposes class names such as `Eq`, `Default`, `Ord`, `Semigroup`, `Monoid`, `Functor`, `Bifunctor`, `Traversable`, `Filterable`, `Applicative`, `Monad`, and `Foldable` through `aivi` and `aivi.prelude`.

Keep the docs conservative: do not invent Haskell-style deriving rules, hierarchy claims, or constraint syntax that is not already exercised by the shipped surface.
