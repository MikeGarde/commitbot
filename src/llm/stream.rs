use anyhow::Result;
use std::io::{self, BufRead, Write};

/// Read a streaming response line-by-line, printing chunks as they arrive.
///
/// When `quiet` is true, chunks are still captured into the returned string
/// but nothing is written to stdout — used for `--oneline quiet`, where only
/// the final one-line summary should be printed.
pub fn read_stream_to_string<R, F>(reader: R, quiet: bool, parse_line: F) -> Result<String>
where
    R: BufRead,
    F: FnMut(&str) -> Result<Option<String>>,
{
    read_stream_to_writer(reader, quiet, &mut io::stdout(), parse_line)
}

/// Same as [`read_stream_to_string`], but echoes chunks to `out` instead of
/// stdout so the quiet/non-quiet behavior can be tested.
fn read_stream_to_writer<R, W, F>(
    reader: R,
    quiet: bool,
    out: &mut W,
    mut parse_line: F,
) -> Result<String>
where
    R: BufRead,
    W: Write,
    F: FnMut(&str) -> Result<Option<String>>,
{
    let mut captured = String::new();

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(chunk) = parse_line(line)? {
            captured.push_str(&chunk);
            if !quiet {
                write!(out, "{}", chunk)?;
                out.flush()?;
            }
        }
    }

    Ok(captured)
}

#[cfg(test)]
mod tests {
    use super::*;
    use anyhow::anyhow;

    /// Treat every non-empty line as a chunk, except `skip` which yields none.
    fn echo_lines(line: &str) -> Result<Option<String>> {
        Ok(if line == "skip" { None } else { Some(line.to_string()) })
    }

    #[test]
    fn echoes_chunks_and_returns_them_when_not_quiet() {
        let mut out = Vec::new();
        let result =
            read_stream_to_writer("one\ntwo\n".as_bytes(), false, &mut out, echo_lines).unwrap();

        assert_eq!(result, "onetwo");
        assert_eq!(String::from_utf8(out).unwrap(), "onetwo");
    }

    #[test]
    fn quiet_captures_chunks_without_writing_anything() {
        let mut out = Vec::new();
        let result =
            read_stream_to_writer("one\ntwo\n".as_bytes(), true, &mut out, echo_lines).unwrap();

        assert_eq!(result, "onetwo");
        assert!(out.is_empty(), "quiet mode must not write: {out:?}");
    }

    #[test]
    fn skips_blank_lines_and_lines_without_a_chunk() {
        let mut out = Vec::new();
        let result = read_stream_to_writer(
            "\n  \none\nskip\n\ntwo\n".as_bytes(),
            false,
            &mut out,
            echo_lines,
        )
        .unwrap();

        assert_eq!(result, "onetwo");
        assert_eq!(String::from_utf8(out).unwrap(), "onetwo");
    }

    #[test]
    fn propagates_parse_errors() {
        let mut out = Vec::new();
        let err = read_stream_to_writer("boom\n".as_bytes(), false, &mut out, |_| {
            Err(anyhow!("bad chunk"))
        })
        .unwrap_err();

        assert!(err.to_string().contains("bad chunk"));
    }

    #[test]
    fn public_wrapper_returns_captured_text() {
        // Quiet so the test doesn't spray into the harness's stdout.
        let result = read_stream_to_string("a\nb\n".as_bytes(), true, echo_lines).unwrap();
        assert_eq!(result, "ab");
    }
}
