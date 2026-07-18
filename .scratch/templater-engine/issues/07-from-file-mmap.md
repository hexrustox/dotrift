# 07 — `from_file` (mmap) construction and byte-accurate render-error miette spans

**What to build:** `Template::from_file<P: AsRef<Path>>(path: P) -> Result<Template>` mmap's the file at the given path and parses it the same way `from_bytes` does, feeding the resulting `&[u8]` into the same parse pipeline. The `Source::Mapped(Mmap)` variant is added beside the existing `Source::Owned(Vec<u8>)`. The `ByteSource` wrapper gains a `Borrowed(&'a [u8])` variant for render errors, borrowing from the existing `Template`'s `Source` (zero-copy) — replacing the `Owned(Arc<[u8]>)` source-duplication pattern earlier tickets used for parse errors. Parse errors still duplicate the source bytes into `Arc<[u8]>` (since the `Template` doesn't exist yet on failure). Construction-time IO errors (file open, mmap) and render-time IO errors (write, flush) both surface as top-level `Error::Io(io::Error)` with no byte span attached. The miette `Report::with_source_code(ByteSource::...)` wrap is wired at both construction and render boundaries — render errors borrow from `&self.source`, parse errors `Owned`-duplicate into the error.

**Blocked by:** 06

**Status:** ready-for-agent

- [ ] `Source::Mapped(Mmap)` variant exists beside `Source::Owned(Vec<u8>)`; `Source::as_bytes() -> &[u8]` returns the appropriate slice for both variants.
- [ ] `Template::from_file<P: AsRef<Path>>(path: P) -> Result<Template>` mmap's the file and parses; mmap or file-open failure surfaces as `Error::Io(io::Error)`.
- [ ] `ByteSource::Borrowed(&'a [u8])` variant implements `miette::SourceCode` (per-span `String::from_utf8_lossy` inside `read_span`); render errors borrow from `&self.source` (zero-copy).
- [ ] Parse errors continue to use `ByteSource::Owned(Arc<[u8]>)` (source duplicated into the error, since the `Template` doesn't exist yet on failure).
- [ ] Render-time IO errors (writer `write_all` or final `flush` failure) surface as `Error::Io(io::Error)`; construction IO errors (file open, mmap) surface the same way.
- [ ] miette `Report::new(err).with_source_code(ByteSource::...)` wired at both construction and render boundaries in `lib.rs`.
- [ ] End-to-end tests in `templater/tests/render.rs` cover `from_file` happy path (template file renders identically to the same bytes via `from_bytes`); error tests in `templater/tests/error.rs` cover file-not-found and mmap-failure paths via `matches!(err, Error::Io(_))`.
- [ ] `cargo test -p templater`, `cargo fmt`, `cargo clippy -p templater` pass.