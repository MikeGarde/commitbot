use anyhow::Result;
use std::io::{self, BufRead, Write};

/// Read a streaming response line-by-line, printing chunks as they arrive.
pub fn read_stream_to_string<R, F>(reader: R, mut parse_line: F) -> Result<String>
where
    R: BufRead,
    F: FnMut(&str) -> Result<Option<String>>,
{
    let mut out = String::new();
    let mut stdout = io::stdout();

    for line in reader.lines() {
        let line = line?;
        let line = line.trim();
        if line.is_empty() {
            continue;
        }

        if let Some(chunk) = parse_line(line)? {
            out.push_str(&chunk);
            print!("{}", chunk);
            stdout.flush()?;
        }
    }

    Ok(out)
}
