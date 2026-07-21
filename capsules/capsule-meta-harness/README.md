# aos-meta-harness

`aos-meta-harness` gives host agents a private, same-turn reminder to notice
reusable capability gaps in their own Unicity AOS user-space world. It answers
only correlated `user_prompt_submit` events, so it cannot create a second turn
or broadcast context to every session owned by the same principal.

The default `adaptive` activation leaves the reuse/build/propose decision to
the agent. Operators may select `propose`, `automatic`, or `off` through the
per-principal capsule configuration.
