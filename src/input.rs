//! Shared bounded UTF-8 input for CLI, MCP stdio and HTTP bodies.
use anyhow::{Result, bail, ensure};
use std::io::{BufRead, Read};

pub const DEFAULT_MAX_BYTES: usize = 1024 * 1024;
pub const HARD_MAX_BYTES: usize = 64 * 1024 * 1024;

pub fn max_bytes() -> Result<usize> {
    match std::env::var("MNEMOSYNE_MAX_INPUT_BYTES") {
        Ok(value) => {
            let value = value.parse::<usize>().map_err(|_| {
                anyhow::anyhow!(
                    "MNEMOSYNE_MAX_INPUT_BYTES must be an integer from 1 to {HARD_MAX_BYTES}"
                )
            })?;
            ensure!(
                (1..=HARD_MAX_BYTES).contains(&value),
                "MNEMOSYNE_MAX_INPUT_BYTES must be from 1 to {HARD_MAX_BYTES}"
            );
            Ok(value)
        }
        Err(std::env::VarError::NotPresent) => Ok(DEFAULT_MAX_BYTES),
        Err(_) => bail!("MNEMOSYNE_MAX_INPUT_BYTES must be valid UTF-8"),
    }
}

pub fn read(reader: impl Read) -> Result<String> {
    read_bytes(reader, max_bytes()?).and_then(|bytes| String::from_utf8(bytes).map_err(Into::into))
}

pub fn read_bytes(reader: impl Read, limit: usize) -> Result<Vec<u8>> {
    ensure!((1..=HARD_MAX_BYTES).contains(&limit), "Invalid input bound");
    let mut bytes = Vec::new();
    reader.take((limit + 1) as u64).read_to_end(&mut bytes)?;
    ensure!(
        bytes.len() <= limit,
        "INPUT_TOO_LARGE: input exceeds {limit} bytes"
    );
    Ok(bytes)
}

/// Stops immediately on overflow, including an unterminated frame. The caller
/// must close this connection rather than try to parse the unread frame tail.
pub fn line(reader: &mut impl BufRead, limit: usize) -> Result<Option<Vec<u8>>> {
    ensure!((1..=HARD_MAX_BYTES).contains(&limit), "Invalid input bound");
    let mut bytes = Vec::new();
    loop {
        let available = reader.fill_buf()?;
        if available.is_empty() {
            return Ok((!bytes.is_empty()).then_some(bytes));
        }
        let end = available.iter().position(|&b| b == b'\n');
        let count = end.unwrap_or(available.len());
        ensure!(
            count <= limit.saturating_sub(bytes.len()),
            "INPUT_TOO_LARGE: stdio frame exceeds {limit} bytes"
        );
        bytes.extend_from_slice(&available[..count]);
        reader.consume(count + usize::from(end.is_some()));
        if end.is_some() {
            return Ok(Some(bytes));
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn bounded_read_does_not_consume_an_unlimited_stream() {
        assert!(
            read_bytes(std::io::repeat(b'x'), 32)
                .unwrap_err()
                .to_string()
                .contains("INPUT_TOO_LARGE")
        );
        let mut reader = std::io::BufReader::with_capacity(8, std::io::repeat(b'x'));
        assert!(
            line(&mut reader, 32)
                .unwrap_err()
                .to_string()
                .contains("INPUT_TOO_LARGE")
        );
        let mut reader = &b"abcd\ne\n"[..];
        assert_eq!(line(&mut reader, 4).unwrap().unwrap(), b"abcd");
        assert_eq!(line(&mut reader, 4).unwrap().unwrap(), b"e");
        assert!(line(&mut reader, 4).unwrap().is_none());
    }
}
