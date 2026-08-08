# ADR-0010: Deploy actions are non-atomic: write, then state, then mode

Each deploy action runs in a fixed order: write the file bytes or create the
symlink, update the management state, then apply the configured mode. There is
no temporary-file-plus-atomic-rename and no rollback. A failure at any step
returns an error and exits, leaving the completed steps in place — a
state-write failure leaves the filesystem change, and a mode-application
failure leaves the new content and state in place with the mode not applied.
This is a deliberate consequence of the reconciliation model: state must mirror
the filesystem as it actually is after the action (ADR-0007), and pretending
otherwise would misclassify the path on the next run. Atomic replacement was
rejected for the complexity it would add across symlinks, directories, and
permissions without changing the core guarantee, which is that a failed action
is reported, never silently retried or rolled back.
