# Profile precedence by activation timestamp

Template variables come from the `[variable]` base plus any number of simultaneously-active profiles in `dotrift_data.toml`. When two active profiles set the same key, one must win.

Precedence is determined by `activated_at` — a millisecond Unix timestamp stored in the `active_profiles` database table. The profile with the highest `activated_at` wins on conflict. Re-activating an already-active profile updates its `activated_at` to now, moving it to the back of the precedence order. Profiles are not ordered by name, list position, or declaration order in `dotrift_data.toml`.

The alternative — positional ordering maintained in database or config — would require a join table, careful update logic for re-activation, and a way to reorder. A timestamp gives a global, monotonic order for free, and re-activation naturally means "this profile should take precedence from now on," which a timestamp bump expresses directly. Two profiles activated within the same millisecond would tie; `activated_at` is taken from system monotonic time and collision risk is negligible, with database row order as the final tiebreaker.
