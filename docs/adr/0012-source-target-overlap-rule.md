# ADR-0012: Source and target roots overlap only as source-inside-target

The source directory may lie inside the target directory — the common layout
of a dotfile tree inside a home root — but the target directory may not equal
the source directory or lie inside it. Equality or target-inside-source would
let deployment target its own control files and sources during the run.
Individual target paths that fall inside the source tree are not additionally
rejected: the roots are checked only at the top level, and entries read source
content at their execution turn, so earlier actions may change a later entry's
source with no special handling beyond the normal runtime failure behavior.
