# Source Maintainability

The selected-definition Analysis view offers an explicit **Measure source**
command. It measures captured function or method syntax, without executing the
repository. Values are not a code-quality score or semantic correctness finding.

The versioned report counts noncomment code lines, lexical tokens, maximum
explicit syntax nesting and unsafe syntax boundaries. Source locations retain
exact UTF-8 byte ranges. Each location category is paged in the browser, and
activating a location opens the corresponding source. The report exposes its
input digest, algorithm version, assumptions and unavailable information.

## Meaning And Limits

- Source lines intersect non-whitespace, non-comment token characters. Comments
  in strings are not stripped; CRLF and multiline literal handling are explicit.
- Lexical tokens come from the pinned rust-analyzer lexer, not splitting text on
  whitespace or treating combined CST punctuation as a lexical token.
- Nesting follows a named syntax traversal, including lexical control, closure,
  nested-function and async boundaries. Ordinary and unsafe blocks are not extra
  nesting levels. This is not a standardized cognitive-complexity measure.
- Unsafe locations describe spelled boundaries and their nearest lexical
  enclosure, not a count of unsafe operations or proof of unsafety.
- Function headers, attributes and nested declarations belong to the selected
  source scope. Expanded/generated code is not inferred. Unknown provenance,
  macro effects and cfg limitations remain visible.

The HTTP endpoint authorizes the snapshot, context and definition before loading
the exact verified source. It accepts complete source files up to 256 KiB, with a
250 ms source-query control and a two-second analysis control. It uses the existing
bounded blocking-worker admission and cancels on request drop. Parser/lexer calls
are not individually preemptible; byte caps bound their input.

Default traversal limits are 20,000 syntax nodes, 50,000 lexical tokens, 1,000
locations per category and 1 MiB response bytes. Interrupted counting withholds
the counts rather than returning a complete prefix. Separately capped location
lists set `locations_truncated`; they do not turn completed counts into zeros.
Parser failures also withhold counts. Inspection results remain separate from
compiler CFG complexity and whole-repository metrics.

No source metric history, inferred rename tracking, normalized-structure
duplication detector, test sufficiency rating or universal ranking is supplied by
this endpoint. Those remain separate specification work.

## Compiler CFG Measure

With a compatible compiler import selected in Flow, **Measure CFG** separately
computes a structural `E - N + 2P` value. It requires the complete imported body,
validates the successor/terminator contract and follows runtime edges from bb0.
Parallel switch alternatives remain distinct edges even when they share a target.
Imaginary edges and false-unwind-only alternatives are excluded.

The versioned convention introduces one synthetic exit. Each block with an
external unwind alternative, or otherwise a terminal leaf, contributes one edge
to that exit. `N` includes that exit and `P` is one for the selected connected
entry-reachable graph. A graph without an exit endpoint has no published value.
A terminal leaf may represent an unreachable point, tail call, assembly or a
nonreturning call; its normalized edge is not a termination claim. Dead blocks
are listed separately. Compiler identity, phase, panic strategy and input digest
remain inspectable; exit locations reuse exact compiler source mappings.

The HTTP limit is 200 blocks and 2,000 normalized edges with a two-second analysis
control and 256 KiB report budget. Complete counts remain separate from withheld
location records. Source syntax counts, compiler CFG structure and observed
execution are distinct evidence categories, not interchangeable complexity scores.
