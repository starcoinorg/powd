# `powd` Identity and Minimal Protocol

## Purpose

This document covers the local identity model used by `powd`.

It answers four questions:

- what the payout wallet address means
- what `worker_name` is responsible for
- how a minimal `agent_auth` handshake can work
- how that handshake relates to the normal Stratum flow

## Core conclusion

The current external identity model keeps only two values:

- `wallet_address`
  - receives payout
- `worker_name`
  - is the stable local `powd` identity

The pool-facing login format stays:

- `wallet_address.worker_name`

That means:

- `wallet_address` handles money
- `worker_name` handles local process identity

## Identity model

### `wallet_address`

`wallet_address` is the payout address.

Requirements:

- it can be created locally
- it does not need a prior Starcoin network round-trip
- at this stage it only carries payout semantics

### `worker_name`

`worker_name` is the stable local identity.

It is created once and persisted locally. Restarts reuse it. It is not a cosmetic label.

It still needs to satisfy current pool worker naming rules:

- lowercase
- letters, digits, `_`, `-`
- no longer than the current server limit

## Minimal protocol

ASIC workers should keep standard Stratum. `powd` is different because both sides are under our control, so a thin `agent_auth` step is acceptable.

Minimal flow:

1. `powd` connects to the agent-facing `stratumd`
1. the server returns a challenge
1. the client replies with:
   - `worker_name`
   - `agent_pubkey`
   - `sig(challenge || worker_name)`
   - optional version
1. the server verifies the signature
1. normal Stratum continues with login:
   - `wallet_address.worker_name`
1. the server requires the `worker_name` in login to match the authenticated one

The only goal is to prove that the connection holds the stable local identity key.
