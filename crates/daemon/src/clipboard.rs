//! Best-effort clipboard copy. Returns Ok(false) when no clipboard tool is
//! available (CI runners, headless Linux, Windows). Never panics.

use std::io::Write;
use std::process::{Command, Stdio};

pub fn copy(text: &str) -> anyhow::Result<bool> {
    #[cfg(target_os = "macos")]
    let (bin, args): (&str, &[&str]) = ("pbcopy", &[]);

    #[cfg(target_os = "linux")]
    let (bin, args): (&str, &[&str]) = ("xclip", &["-selection", "clipboard"]);

    #[cfg(not(any(target_os = "macos", target_os = "linux")))]
    {
        let _ = text;
        return Ok(false);
    }

    #[cfg(any(target_os = "macos", target_os = "linux"))]
    {
        let mut child = match Command::new(bin)
            .args(args)
            .stdin(Stdio::piped())
            .stderr(Stdio::null())
            .stdout(Stdio::null())
            .spawn()
        {
            Ok(c) => c,
            Err(_) => return Ok(false),
        };
        if let Some(stdin) = child.stdin.as_mut() {
            stdin.write_all(text.as_bytes())?;
        }
        let status = child.wait()?;
        Ok(status.success())
    }
}
