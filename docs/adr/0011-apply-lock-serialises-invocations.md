# ADR-0011: Concurrent `apply` invocations are serialised by an exclusive lock

`apply` acquires an exclusive apply lock before reading the control files and
holds it through preflight, prompts, filesystem actions, state updates, and
exit. A concurrent invocation that cannot acquire the lock fails through the
normal command error path rather than interleaving operations. Without the
lock, two processes could inspect the same fingerprint, both decide a path is
managed, overwrite each other, and commit inconsistent state. Relying on the
state database's own transactions alone would still leave an inspect-then-act
gap across the whole reconcile loop. Waiting for the lock was rejected because
a blocked invocation is indistinguishable from a hang from the user's
perspective; a failing second invocation fails fast and is actionable.
