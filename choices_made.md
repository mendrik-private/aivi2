# Choices made for the initial implementation wave

This file is a plain-language summary of the current implementation choices, with one short explanation per section.

1. **Validation notation:** Keep the behavior the same, but describe it using AIVI's own terms instead of Haskell terms.

2. **`?|>` gate behavior:** This operator means "keep it if the check passes." For changing or live values, only passing updates continue and failures do not create fake opposite updates.

3. **Reactive text inside `@source`:** If a source string includes changing values, those values count as dependencies. When they change, the source is rebuilt using the new text.

4. **HTTP refreshes:** HTTP sources refresh only for explicit reasons, such as changing reactive inputs or timers or retries written in code. There are no hidden refreshes tied to focus, windows, or other app lifecycle events.

5. **Watching files vs reading files:** Watching a file only reports that something changed. Reading and decoding the file must still be done as a separate step.

6. **Orphan instances:** These are fully disallowed for now to keep behavior consistent and easy to reason about.

7. **Milestone order:** Build the system in milestone order and keep each layer responsible for its own job. Later runtime concerns should not leak back into earlier compiler stages.

8. **Advanced type features:** Start with a smaller, predictable set of advanced type features. More powerful type-level features can be added later as a separate design.

9. **`<each>` keys:** Every `<each>` must have a key. This keeps repeated UI items stable when they move or change.

10. **Decoder overrides:** Only the built-in decoding path exists for now. Custom decoder hooks are delayed until the base behavior is fully defined.

11. **Internal data layout:** Use stable internal IDs and iterative walking instead of deep recursion. This makes the compiler safer and more reliable on large inputs.

12. **Equality support:** Equality is provided automatically only for data shapes that are clearly and safely comparable. Functions, live values, tasks, and outside handles are excluded.

13. **Early internal program model:** The early compiler model uses stable IDs for items, bindings, and expressions, and it keeps source locations attached. That makes later processing and error reporting more reliable.

14. **Record shorthand:** Record shorthand stays in shorthand form during early processing. This preserves better errors and avoids pretending the compiler already knows the full record shape.

15. **Applicative clusters:** These special pipe clusters stay grouped together instead of being flattened too early. That keeps their original meaning and diagnostics intact.

16. **Special markup tags:** Tags like `show`, `each`, and `match` are treated as real control features, not normal markup. This makes later UI handling much simpler.

17. **Decorators:** Only `@source` is a real decorator right now. Other decorator-like syntax is rejected instead of being carried around as unknown metadata.

18. **Name lookup and imports:** Name lookup stays intentionally simple: local names and a small set of imports work, but there are no aliases, wildcards, or module-qualified names yet. Built-in names also keep priority where needed.

19. **Recent spec updates:** Recent spec updates change what later stages must do, but they do not force a redesign of the current early compiler shape. `domain` will be added as its own feature instead of being squeezed into older type handling.

20. **`domain` declarations:** A `domain` declaration is treated as its own real language feature. You can declare domain suffixes now, but using them directly in expressions is handled later.

21. **Equality for domains:** Domains can automatically get equality when their underlying value can be compared. Even so, they still remain their own named types.

22. **Suffix literals like `250ms`:** Compact forms like `250ms` are treated as special suffix literals, while spaced forms like `250 ms` keep their normal meaning. Only integer-based suffixes are supported for now.

23. **Where suffixes are resolved:** Literal suffixes are resolved only against declarations in the current module for now. No match is an error, and multiple matches are treated as ambiguous.

24. **Newer pipe operator rules:** Only the obvious shape and ordering rules for the newer pipe operators are enforced at this stage. Deeper behavior that depends on typing or runtime rules is left for later.

25. **Interpolated text structure:** Interpolated text is stored as alternating plain text and expression holes instead of one opaque string. That makes dependency tracking and error reporting clearer.

26. **Source dependency tracking:** Source-backed signals record which local signals they depend on. Imported references are not guessed to be signals yet.

27. **General signal dependency tracking:** All signals, not just source-backed ones, now carry an explicit list of local signal dependencies. This gives later scheduling work one consistent dependency story.

28. **Early `@source` errors:** The compiler now catches obviously malformed `@source` declarations early, such as missing variants or duplicate options. More detailed provider-specific checks still come later.

29. **Built-in source options:** Built-in sources now have a known list of allowed option names using clearer names like `timeout` and `refreshEvery`. The compiler checks the option names now, but not yet whether each value has the perfect type.

30. **Unfinished applicative clusters:** These shapes are no longer rejected too early by the parser. They are accepted first and then flagged later in the more appropriate validation step.

31. **Type-shape checking foundation:** The project now has a reusable foundation for checking whether advanced type constructors are being used in the right shape. This is groundwork for later type-checking.

32. **Using that checking in early validation:** That new checking is now used in places where the compiler already has enough trustworthy information. Imported types are still skipped until the import system is richer.

33. **Repeating-flow syntax rules:** Repeating-flow syntax is limited to one narrow, clearly structured trailing form for now. Mixed or messy shapes are rejected.

34. **Internal view of applicative clusters:** The compiler keeps these clusters in their user-facing form, but also records a clean internal recipe for what they mean. Later stages can use that recipe without re-guessing it.

35. **Internal view of repeating-flow tails:** Repeating-flow syntax stays visible in the early internal model, but the compiler also exposes a clean extracted view of the repeating tail. Later stages can use that directly instead of rebuilding it by hand.

36. **Catalog of source option shapes:** There is now a central catalog describing the expected shape of built-in source options. That gives later checking a single source of truth.

37. **Gate behavior checks:** Gate behavior is checked using only type facts the compiler can already prove today. Obvious mistakes are rejected, while uncertain cases are left open instead of over-restricted.

38. **Where repeating flows are allowed:** Repeating flows are allowed only where the compiler can already prove they target something supported, such as a signal or task declaration. Everything else is rejected instead of guessed.

39. **Required trigger for repeating flows:** Every repeating flow must have a clear trigger the compiler can already recognize, like a built-in timer or event source. Cases without a provable trigger are rejected for now.

40. **Resolving source option types:** Source option schemas are now matched to real program types where possible. The compiler still stops short of fully type-checking the option values themselves.

41. **Lowering plan for gates:** The compiler now produces a clear lower-level plan for how gate stages should behave later. If it cannot prove enough today, it records the blocker instead of making something up.

42. **Runtime handoff for signal filters:** Signal-based filters now lower into a simple typed filter description that future runtime code can use. Only clearly safe expression forms are included for now.
