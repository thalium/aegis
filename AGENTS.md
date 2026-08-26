# State transport invariants

Aegis transports CPU state over a constrained shared-memory protocol. Storage
and bandwidth are expensive: keep exactly one canonical physical
representation of each architectural state and derive views at use sites.

Do not store or serialize redundant logical/derived fields alongside their
source representation. In particular:

- x87/MMX payloads are physical `R0`–`R7`; derive `ST(i)` from TOP.
- TOP is bits 11–13 of `x87_status`; do not record `x87_top` in results.
- MMX is the low-64 view of physical R slots; do not add independent MM state.

Compatibility inputs may be accepted only at explicit conversion/validation
boundaries. They must not become persistent protocol fields.
