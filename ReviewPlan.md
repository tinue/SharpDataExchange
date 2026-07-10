# Review Plan — SharpDataExchange (2026-07-03)

Findings from a full cross-project code review (see `../SharpBasicShared/ReviewPlan.md`
for the shared-library side). Overall: the project is in good shape — clear verb-based
CLI, clean layering (cli / convert / detect / header / io / serial), 103 passing tests.
Its use of the shared library is the right *kind* of coupling; the issues below are a
stale version pin, one missing error check, and one piece of logic that belongs in the
shared library.

## P1 — Must fix

### 1.1 Stale shared-library version
`pom.xml` pins `version.sharpbasic = 1.0.0-SNAPSHOT`. SharpBasicShared released `1.0.0`
(the SNAPSHOT in `~/.m2` is a March 15 build), so this project silently compiles against
a 3½-month-old shared library and will never see current fixes.
**Fix:** set `version.sharpbasic` to `1.0.0` (and bump when shared 1.1.0 ships with the
detokenizer/encoder fixes, which directly affect this project's `get`/`put` paths).

### 1.2 `AsciiBasicTokenizer` ignores parse errors
`AsciiBasicTokenizer.tokenize()` never checks `parser.getNumberOfSyntaxErrors()`. A BASIC
file with a syntax error is encoded from ANTLR's error-recovered tree and *sent to the
device* as a silently corrupted program.
**Fix:** fail with a clear message (file + error count) before encoding. Once the shared
library exposes its planned pipeline façade (shared ReviewPlan §2.1), use that and this
class shrinks to a few lines.

## P2 — Should fix

### 2.1 `ReserveAreaConverter` duplicates token-scanning logic, incompletely
`toAscii()` and `detokenizeContent()` hard-code `0xF0`/`0xF1` as the only token high
bytes. CE-150/CE-158 keywords use `0xE6/0xE7/0xE8` (e.g. `CSIZE` = 0xE680) and PC-1600
adds `0xE3/0xF2` — reserve-key content containing those tokens is mis-decoded into raw
bytes, and the pool parser may then misinterpret following bytes. `tokenizeContent()` can
*produce* such tokens (it emits whatever the registry returns), so a written pool may not
round-trip through `toAscii()`.
**Fix:** get the valid high-byte set from the registry (shared ReviewPlan §1.1 proposes
`KeywordRegistry.tokenHighBytes()`); ideally move the generic "keyword-token stream ↔
text" scan into the shared library and keep only the reserve-area pool structure here.
Add a round-trip test with a CE-150 keyword in a reserve key.

### 2.2 PC-1600 detokenization is broken upstream
`get` in ASCII format for a PC-1600 BASIC program goes through the shared
`BinaryBasicDetokenizer`, which cannot decode `0xE3xx`/`0xF2xx` tokens (shared ReviewPlan
§1.1). No local change needed, but add a PC-1600 fixture test here once shared 1.1.0 is in.

## P3 — Minor

- `SharpDataExchange.runTerminal`: `InterruptedException` from `Thread.sleep(10)` is
  swallowed by `catch (Exception)` without re-interrupting the thread.
- `encodeAsciiBasic/…Reserve/…Vars` use `args.device()` while the surrounding `runPut`
  computes `effectiveDevice`; today they coincide for ASCII input (header inference only
  happens for binary input), but passing `effectiveDevice` through would make that
  invariant explicit.
- The `catch (Exception e)` in `main` logs only `e.getMessage()` — for unexpected
  exceptions (NPE) that logs "Fatal error: null". Log the exception object at FINE/SEVERE
  so `-vv` reveals the stack trace.
