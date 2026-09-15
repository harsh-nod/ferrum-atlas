# State-machine syntax candidates

This opt-in analysis searches one exact captured function or method for a selected
enum path and a simple local variable. It never runs the repository or expands
macros. The pinned rust-analyzer syntax parser supplies the AST; string search is
not used to identify transitions.

## Supported boundary

A candidate requires a direct top-level `match state`, an explicit unit variant
pattern such as `State::Idle`, and one direct assignment such as
`state = State::Ready`. The assignment can be enclosed in an otherwise empty
block. The selected local must have one explicit non-reference parameter or
top-level local binding. Paths are syntax spellings, not resolved enum or binding
identities. Guards containing simple expression syntax are retained verbatim;
their truth, purity, trait dispatch and feasibility are not established.

Every candidate includes absolute UTF-8 byte spans for the match, arm, assignment,
guard and action. Guard and action excerpts are exact captured text, each limited
to 4096 bytes. Source offsets include any prefix before the selected definition;
CRLF and Unicode are not normalized. A supplied body span must exactly match the
parsed function body. Inactive or unknown cfg contexts yield no candidates.

Nested matches, wildcard and compound patterns, payload variants, member places,
dereferences, multiple actions, explicit calls or assignments in guards, and
attributed arms are unknown. Operator or dereference trait effects are not
resolved, even in guards whose syntax is supported.
Attributes anywhere in the function (including parameters), macros, references
or aliases of the selected local, and shadowed or ambiguous
bindings withhold all candidates. Other unmodeled statements remain explicit
unknowns. This is intentionally conservative and may omit valid transitions.

Coverage is always partial, including when no candidates or unknown records are
returned. Candidates do not imply an exhaustive state set, feasible execution,
complete transitions, or whole-program behavior. No complete negative answer is
available. Multiple matches are independent syntactic sites, not a reconstructed
execution schedule.

Parsing uses Rust edition 2024. Definition metadata does not supply an edition
to this pure API; compatibility with another context edition is not established.

## Budgets

Defaults are 262144 selected definition bytes, 20000 AST nodes, 200 candidates,
100 unknown records and 1048576 serialized response bytes. Hard maxima are 1 MiB,
100000 nodes, 500 candidates, 500 unknown records and 2 MiB. The response minimum
is 1024 bytes; a budget unable to hold even the envelope fails explicitly.
Selected definition metadata is limited to 64 KiB before digesting or parsing.
The source limit conservatively includes the function header and attributes.
The parser is bounded by source bytes; cancellation is checked before parsing,
throughout the binding prepass and between candidate arms. Parsing itself is not
preemptible. A stopped or node-limited prepass emits no candidates, since unseen
bindings could invalidate local-identity assumptions. Later cutoffs retain only
the observed prefix with truncation set. Response overflow withholds candidate
and unknown records, preserving the partial budget envelope or failing if it
cannot fit. Empty withheld records must not be displayed as a count of zero
possible transitions.

## Review declarations

Inference and review are separate records. The input digest binds algorithm
version, exact selected source bytes, complete definition metadata (including
context), selected path spellings and all limits. Each candidate ID also binds
its exact variants, spans, guard and action. Changing any bound input invalidates
an old review; cancellation can withhold a candidate but cannot authorize its
review. The enclosing API additionally pins snapshot and context.

A review names the exact input digest and candidate ID, an `accepted` or
`rejected` decision, a reviewer and an optional explanatory note (represented as
an empty string when absent). Reviewer text must be nonblank, at most 128 UTF-8
bytes and contain no control characters. Notes are at most 4096 UTF-8 bytes;
only tab, CR and LF controls are permitted. Validation checks candidate identity
and digest binding. It does not authenticate the reviewer name; that is the
responsibility of the enclosing service and its authentication policy.

An accepted declaration is not a proof, does not upgrade coverage, and does not
turn unknown or missing transitions into impossible ones. The analysis library
does not persist or merge reviews. The service owns separately scoped immutable
review storage, authorization and retention.

## Verification

The independent hand-authored fixture oracle asserts exact transition targets
and source spans. Deliberately changing a target changes the oracle result and
digest; changing the assignment to another variable or enum removes that
candidate rather than fabricating a resolution. Tests cover syntax exclusions,
aliases, shadowing, macros, inactive contexts, UTF-8 and CRLF spans, each output
budget, cancellation, determinism, review metadata and stale or forged reviews.
These tests qualify the documented syntax boundary only, not inferred semantic
correctness, human review quality or large-repository acceptance.
