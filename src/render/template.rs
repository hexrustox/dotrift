use std::{
    collections::HashMap,
    io::{self, Write},
    path::Path,
};

use miette::{Report, Result, WrapErr, miette};
use templater::{Template, value::Value};

use super::Builtins;

/// Why rendering a template stopped: the template itself, or the sink it was
/// rendered into.
pub(crate) enum RenderFailure {
    /// Reading the source, parsing, or rendering failed; the run must report
    /// this. Carries the final diagnostic.
    Template(Report),
    /// Writing the rendered bytes failed; infrastructure, not the template.
    Sink(io::Error),
}

/// Renders the template at `path` into `writer`, reporting whether the
/// template or the sink failed.
pub(crate) fn render_template_into(
    path: &Path,
    context: &HashMap<String, Value>,
    writer: impl Write,
) -> std::result::Result<(), RenderFailure> {
    let template = Template::from_file(path)
        .map_err(|error| miette!(error))
        .wrap_err_with(|| format!("cannot read template `{}`", path.display()))
        .map_err(RenderFailure::Template)?;
    let result = template.render(writer, context, &Builtins);
    match result {
        Ok(()) => Ok(()),
        Err(templater::error::Error::Io(error)) => Err(RenderFailure::Sink(error)),
        Err(error) => template
            .report(Err(error))
            .map_err(|error| miette!(error))
            .wrap_err_with(|| format!("cannot render template `{}`", path.display()))
            .map_err(RenderFailure::Template),
    }
}

/// Renders the template at `path` into `writer`, treating a sink failure as
/// fatal like any other rendering failure.
pub(crate) fn render_template_to(
    path: &Path,
    context: &HashMap<String, Value>,
    writer: impl Write,
) -> Result<()> {
    match render_template_into(path, context, writer) {
        Ok(()) => Ok(()),
        Err(RenderFailure::Template(report)) => Err(report),
        Err(RenderFailure::Sink(error)) => {
            Err::<(), _>(miette!(error))
                .wrap_err_with(|| format!("cannot render template `{}`", path.display()))?;
            unreachable!()
        }
    }
}

pub(crate) fn render_template(path: &Path, context: &HashMap<String, Value>) -> Result<Vec<u8>> {
    let mut output = Vec::new();
    render_template_to(path, context, &mut output)?;
    Ok(output)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;

    use tempfile::tempdir;

    use super::*;

    fn context() -> HashMap<String, Value> {
        HashMap::from([("str".to_string(), Value::Str("str".into()))])
    }

    struct FailingWriter;

    impl io::Write for FailingWriter {
        fn write(&mut self, _: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn template_in(dir: &tempfile::TempDir, contents: &[u8]) -> std::path::PathBuf {
        let path = dir.path().join("file1");
        fs::write(&path, contents).expect("cannot write template");
        path
    }

    #[test]
    fn render_template_renders_the_variables() {
        let dir = tempdir().expect("cannot create temp dir");
        let path = template_in(&dir, b"{{ str }}\n");
        let output = render_template(&path, &context()).expect("cannot render template");
        assert_eq!(output, b"str\n");
    }

    #[test]
    fn render_template_calls_builtins() {
        let dir = tempdir().expect("cannot create temp dir");
        let path = template_in(&dir, b"{{ upper(str) }}-{{ length(str) }}\n");
        let output = render_template(&path, &context()).expect("cannot render template");
        assert_eq!(output, b"STR-3\n");
    }

    #[test]
    fn an_undefined_builtin_call_fails_as_a_template_failure() {
        let dir = tempdir().expect("cannot create temp dir");
        let path = template_in(&dir, b"{{ nope() }}\n");
        let error =
            render_template(&path, &context()).expect_err("an undefined function must fail");
        assert!(error.to_string().contains("cannot render"), "{error}");
    }

    #[test]
    fn a_missing_template_fails_as_a_template_failure() {
        let dir = tempdir().expect("cannot create temp dir");
        let path = dir.path().join("file1");
        let error = render_template_into(&path, &context(), Vec::new())
            .expect_err("missing template must fail");
        assert!(matches!(error, RenderFailure::Template(_)));
    }

    #[test]
    fn a_malformed_template_fails_as_a_template_failure() {
        let dir = tempdir().expect("cannot create temp dir");
        let path = template_in(&dir, b"{{ str\n");
        let error = render_template_into(&path, &context(), Vec::new())
            .expect_err("malformed template must fail");
        assert!(matches!(error, RenderFailure::Template(_)));
    }

    #[test]
    fn a_failing_sink_is_reported_as_a_sink_failure() {
        let dir = tempdir().expect("cannot create temp dir");
        let path = template_in(&dir, b"{{ str }}\n");
        let failure = render_template_into(&path, &context(), FailingWriter)
            .expect_err("a failing sink must fail");
        assert!(matches!(failure, RenderFailure::Sink(_)));
    }

    #[test]
    fn a_sink_failure_fails_like_a_rendering_failure() {
        let dir = tempdir().expect("cannot create temp dir");
        let path = template_in(&dir, b"{{ str }}\n");
        let error = render_template_to(&path, &context(), FailingWriter).expect_err("sink failure");
        assert!(error.to_string().contains("cannot render"), "{error}");
    }

    #[test]
    fn a_template_failure_fails_with_its_report() {
        let dir = tempdir().expect("cannot create temp dir");
        let path = dir.path().join("file1");
        let error =
            render_template_to(&path, &context(), Vec::new()).expect_err("missing template");
        assert!(
            error.to_string().contains(&path.display().to_string()),
            "{error}"
        );
    }
}
