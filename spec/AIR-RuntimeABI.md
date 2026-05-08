# AIR Runtime ABI

## Capability Interface

```
air.capability @email.send {
  inputs(
    %world: !air.world,
    %auth: !air.auth<action = @email.send, resource = %draft>,
    %draft: !air.email_draft
  )
  outputs(
    %msg: !air.sent_message,
    %world1: !air.world
  )
  effects [
    #air.effect<external_communication>,
    #air.effect<network>,
    #air.effect<irreversible>
  ]
  failures [
    #air.fail<network_timeout>,
    #air.fail<permission_denied>,
    #air.fail<recipient_invalid>
  ]
}
```

- Providers implement `describe`, `validate`, `invoke`, `dry_run`, `rollback`, `subscribe`, `explain`.
- Runtime enforces authority tokens and policies before invocation.

## Sandbox Runtime

- Fake providers for email, calendar, filesystem, model, human, policy, clock.
- Sandboxes isolate effect domains and enforce capability manifests.

## ABI Conventions

- Arguments passed as structured values; resources stay linear.
- Failures return tagged unions representing declared failure modes.
- Runtime logs every invoke and result in trace.
