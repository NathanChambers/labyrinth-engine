# System name

**Status:** Proposed or implemented  
**Owner:** Crate or component responsible for authoritative state  
**Source:** Links to relevant paths when implemented

## Purpose and boundary

What the system does, what it owns, and what it leaves to other systems.

## State and invariants

Authoritative state, who may change it, and rules that must always hold. Include persistent or serialized forms when present.

## Public contract

Inputs, outputs, commands, events, and guarantees visible to consumers. Link to a versioned interface when one exists.

## Lifecycle

Initialization, update timing, reload or reconnection, failures, and teardown where relevant.

## Connections

| Other system | Direction | Data or command | When | Failure behavior |
| --- | --- | --- | --- | --- |

Describe the end-to-end path for one meaningful interaction when a table alone is not enough.

## Verification and open questions

Link to checks that establish behavior. Label unverified assumptions and decisions still open.
