# 03 — Configuration and Desired Deployment

**What to build:** Dotrift reads the control files and produces a validated desired deployment from portals, ignore patterns, and Rules before touching the target directory.

**Blocked by:** 02 — Profiles and Variable Context

**Status:** ready-for-agent

- [ ] Rendered `dotrift.toml`, plain `dotrift_data.toml`, and plain `.dotriftignore` follow their discovery and validation rules.
- [ ] Literal and glob portals resolve source paths, descendants, and stripping prefixes correctly.
- [ ] Ignore patterns filter resolved target paths in order, including implicit control-file exclusions and negation.
- [ ] Rules resolve deploy types and modes with declaration-order property precedence.
- [ ] Collisions, structural conflicts, unsafe paths, invalid rules, invalid sources, and source/target overlap fail before filesystem changes.
