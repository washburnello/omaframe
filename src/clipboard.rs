//! Clipboard: `wl-copy` with an OSC52 fallback (plan §5).
//!
//! `Copy` button + `Ctrl-K → Copy selection as text` is the
//! paste-into-agent flow. On Wayland/Omarchy [`copy_text`] prefers the
//! native `wl-copy` binary; anywhere else (SSH, tmux without Wayland,
//! missing `wl-copy`) it emits an OSC52 escape to stdout so a
//! capable terminal still receives the text.

/// Copy `s` to the clipboard.
///
/// Tries `wl-copy` (stdin-piped) first; on any failure (binary missing,
/// non-zero exit, IO error) falls back to writing an OSC52
/// (`ESC ] 52 ; c ; <base64> BEL`) sequence to stdout.
///
/// Returns `Ok` naming the method actually used (`"wl-copy"` or
/// `"osc52"`), or `Err` describing the failure when both paths fail.
pub fn copy_text(s: &str) -> Result<String, String> {
    match try_wl_copy(s) {
        Ok(()) => Ok("wl-copy".to_string()),
        Err(wl_err) => match try_osc52(s) {
            Ok(()) => Ok("osc52".to_string()),
            Err(osc_err) => Err(format!(
                "clipboard failed: wl-copy ({wl_err}) and OSC52 ({osc_err}) both failed"
            )),
        },
    }
}

/// Pipe `s` through the external `wl-copy` binary (no new crates:
/// `std::process::Command` only).
fn try_wl_copy(s: &str) -> Result<(), String> {
    use std::io::Write;
    use std::process::{Command, Stdio};

    let mut child = Command::new("wl-copy")
        .stdin(Stdio::piped())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|e| e.to_string())?;

    if let Some(stdin) = child.stdin.as_mut() {
        stdin.write_all(s.as_bytes()).map_err(|e| e.to_string())?;
    }
    // Close the pipe so `wl-copy` sees EOF before we wait.
    drop(child.stdin.take());
    let status = child.wait().map_err(|e| e.to_string())?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("wl-copy exited with {status}"))
    }
}

/// Write an OSC52 clipboard escape to stdout.
///
/// `ESC ] 52 ; c ; <base64(text)> BEL` — understood by Ghostty,
/// Kitty, Foot, WezTerm, and most modern terminals (also over SSH).
fn try_osc52(s: &str) -> Result<(), String> {
    use std::io::Write;

    let b64 = base64_encode(s.as_bytes());
    let seq = format!("\x1b]52;c;{b64}\x07");
    let mut out = std::io::stdout().lock();
    out.write_all(seq.as_bytes())
        .map_err(|e| e.to_string())?;
    out.flush().map_err(|e| e.to_string())?;
    Ok(())
}

/// Standard base64 (RFC 4648 §4) without new dependencies.
fn base64_encode(input: &[u8]) -> String {
    const ALPHABET: &[u8; 64] =
        b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity(input.len().div_ceil(3) * 4);
    for chunk in input.chunks(3) {
        // `chunks(3)` never yields an empty slice, so `[0]` is safe.
        let b0 = chunk[0] as u32;
        let b1 = chunk.get(1).copied().unwrap_or(0) as u32;
        let b2 = chunk.get(2).copied().unwrap_or(0) as u32;
        let n = (b0 << 16) | (b1 << 8) | b2;
        out.push(ALPHABET[((n >> 18) & 63) as usize] as char);
        out.push(ALPHABET[((n >> 12) & 63) as usize] as char);
        if chunk.len() > 1 {
            out.push(ALPHABET[((n >> 6) & 63) as usize] as char);
        } else {
            out.push('=');
        }
        if chunk.len() > 2 {
            out.push(ALPHABET[(n & 63) as usize] as char);
        } else {
            out.push('=');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn base64_vectors() {
        assert_eq!(base64_encode(b""), "");
        assert_eq!(base64_encode(b"f"), "Zg==");
        assert_eq!(base64_encode(b"fo"), "Zm8=");
        assert_eq!(base64_encode(b"foo"), "Zm9v");
        assert_eq!(base64_encode(b"foob"), "Zm9vYg==");
        assert_eq!(base64_encode(b"fooba"), "Zm9vYmE=");
        assert_eq!(base64_encode(b"foobar"), "Zm9vYmFy");
        assert_eq!(base64_encode(b"hello"), "aGVsbG8=");
    }

    #[test]
    fn copy_text_reports_a_method() {
        // Either backend counts: wl-copy on Wayland/Omarchy, OSC52
        // elsewhere (the fallback writes an escape to the captured test
        // stdout, which is harmless). Must never panic.
        let method = copy_text("omaframe clipboard probe").expect("copy_text");
        assert!(
            method == "wl-copy" || method == "osc52",
            "unexpected method {method:?}"
        );
    }
}
