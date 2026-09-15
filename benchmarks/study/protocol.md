# Human Comprehension Study Protocol

Status: protocol only. No participants recruited and no participant results exist.
The empty CSV is deliberately rejected as study evidence by `validate.py study`.

Recruitment and data collection require a separate authorized study task. Recruit
at least eight consenting adult Rust developers spanning the intended experience
range. Record experience, consent, compensation, and session audit evidence in a
private study log, not this public repository. Use pseudonymous participant IDs.
An agent, generated transcript, or repeated run by one developer is not a distinct
human participant. The validator cannot prove that a person or consent exists;
an independent reviewer must audit the private records.

Use the pinned public corpus or matched small task fixtures that participants
have not previously inspected. Freeze candidate build, browser, baseline editor,
rust-analyzer/toolchain versions, machine, and task-answer rubrics before data
collection. Do not modify task answers after observing a tool's performance.

Each participant completes U1-U10 under both conditions, using matched variants
A and B to reduce memorization. Assign half to Atlas first/variant A, then baseline
variant B; half to baseline first/variant A, then Atlas variant B. Each order group
must contain at least four participants. Counterbalance task order within each
condition using a published seeded Latin-square schedule. Provide equal neutral
training and allow a break between conditions. Baseline includes source search,
editor references, and rust-analyzer navigation; record unavailable capabilities.

Task families follow the specification: U1 entry and reading order; U2 callers
and callees; U3 function flow; U4 state/value dependencies; U5 PR change impact;
U6 insufficiently understood or tested behavior; U7 divergent executions;
U8 maintenance difficulty and structural concentration; U9 selected firmware
build membership; U10 revision-pinned sharing.
Before testing, provide two concrete variants and independently reviewed expected
answers for each family. Unsupported tasks remain in the report as unsupported,
not removed from the denominator. Stop at a predeclared time cap and retain failed,
timed-out, and incorrect answers.

Record elapsed seconds, correctness against the frozen rubric, confidence (1-5),
navigation actions, and brief qualitative notes. The CSV schema requires one row
per participant/task/condition, real-human attestation, consent, and consistent
counterbalance assignment. Archive screen recordings privately only with consent.

Report participant-level paired correctness and time, per-task distributions,
timeouts, experience mix, learning effects, and uncertainty intervals. Do not
collapse correctness and speed into a single favorable number. Eight participants
is an entry gate, not proof of general superiority; publish limitations and avoid
population-level claims that the study is underpowered to support.

The exploratory target is at least 25% lower median task time on U1/U2/U5 with
no reduction in answer accuracy. Report the actual effect and uncertainty even
when the target is missed; meeting it does not by itself establish superiority.
