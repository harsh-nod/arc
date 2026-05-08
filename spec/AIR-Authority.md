# AIR Authority Model

## Purpose

Authority tokens encode the right to perform privileged actions. They are linear values that must dominate their use sites and expire according to declared policy.

## Structure

```
!air.auth<
  principal = %user,
  action = @email.send,
  resource = %draft,
  valid_until = %deadline,
  trust = #air.trust<approved>
>
```

- `principal`: identity responsible for issuing/using the token.
- `action`: capability symbol authorized.
- `resource`: resource or object the authority is scoped to.
- `valid_until`: index or timestamp describing expiration.
- `trust`: trust label.

## Lifecycle

1. **Creation** via operations such as `air.require_approval`, `air.grant_authority`.
2. **Use** when invoking capabilities or effectful operations that require authority.
3. **Consumption** is linear. Authority tokens cannot be duplicated unless the action is declared duplicable.
4. **Revocation** invalidates outstanding tokens; verifier ensures revocation is observed before use.

## Verification Rules

- Authority must dominate uses in control-flow graph.
- Authority tokens with deadlines must be proven valid at use; otherwise insert runtime check.
- Authority tokens are resources; dropping them requires explicit `air.discard_authority`.
- Authority trust levels cannot be escalated without proof.

## Runtime Representation

- Interpreter models authorities as structured records validated at invoke time.
- Trace records issuance and consumption for audit.
- Native runtime may map authorities to OS handles, API tokens, or capability descriptors.
