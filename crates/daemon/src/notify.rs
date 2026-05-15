use std::io::ErrorKind;

use anyhow::Result;
use tracing::warn;

pub trait CommandRunner: Send + Sync {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<()>;
}

pub struct RealRunner;

impl CommandRunner for RealRunner {
    fn run(&self, program: &str, args: &[&str]) -> std::io::Result<()> {
        std::process::Command::new(program)
            .args(args)
            .status()
            .map(|_| ())
    }
}

pub fn notify_with<R: CommandRunner>(runner: &R, title: &str, body: &str) -> Result<()> {
    // Strip double-quotes to prevent injection in osascript -e argument.
    let safe_title = title.replace('"', "");
    let safe_body = body.replace('"', "");

    let result = {
        #[cfg(target_os = "macos")]
        {
            let script = format!(
                "display notification \"{}\" with title \"{}\"",
                safe_body, safe_title
            );
            runner.run("osascript", &["-e", &script])
        }
        #[cfg(not(target_os = "macos"))]
        {
            runner.run("notify-send", &[safe_title.as_str(), safe_body.as_str()])
        }
    };

    match result {
        Ok(()) => Ok(()),
        Err(e) if e.kind() == ErrorKind::NotFound => {
            warn!("notify binary not found");
            Ok(())
        }
        Err(e) => Err(e.into()),
    }
}

pub fn notify(title: &str, body: &str) -> Result<()> {
    notify_with(&RealRunner, title, body)
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io;
    use std::sync::{Arc, Mutex};

    struct StubRunner {
        error: Option<ErrorKind>,
        calls: Arc<Mutex<Vec<(String, Vec<String>)>>>,
    }

    impl StubRunner {
        fn ok() -> (Self, Arc<Mutex<Vec<(String, Vec<String>)>>>) {
            let calls = Arc::new(Mutex::new(vec![]));
            (
                Self {
                    error: None,
                    calls: calls.clone(),
                },
                calls,
            )
        }

        fn not_found() -> Self {
            Self {
                error: Some(ErrorKind::NotFound),
                calls: Arc::new(Mutex::new(vec![])),
            }
        }
    }

    impl CommandRunner for StubRunner {
        fn run(&self, program: &str, args: &[&str]) -> io::Result<()> {
            self.calls.lock().unwrap().push((
                program.to_string(),
                args.iter().map(|s| s.to_string()).collect(),
            ));
            match self.error {
                None => Ok(()),
                Some(kind) => Err(io::Error::from(kind)),
            }
        }
    }

    #[test]
    fn test_notify_success_returns_ok() {
        let (runner, calls) = StubRunner::ok();
        let result = notify_with(&runner, "organism: test", "suggestion ready");
        assert!(result.is_ok());
        assert_eq!(calls.lock().unwrap().len(), 1);
    }

    #[test]
    fn test_notify_missing_binary_returns_ok() {
        let runner = StubRunner::not_found();
        let result = notify_with(&runner, "organism: test", "suggestion ready");
        assert!(result.is_ok(), "missing binary must not propagate error");
    }

    #[test]
    fn test_notify_strips_quotes_from_content() {
        // User-supplied `"` chars must be removed from title/body before being
        // passed to the subprocess (prevents injection in osascript -e string).
        let (runner, calls) = StubRunner::ok();
        notify_with(
            &runner,
            "title with \"quotes\"",
            "body with \"quotes\"",
        )
        .unwrap();
        let locked = calls.lock().unwrap();
        let (_, args) = &locked[0];
        let joined = args.join(" ");
        // The original quoted word must not appear with its surrounding quotes.
        assert!(
            !joined.contains("\"quotes\""),
            "user-supplied quotes survived into args: {joined}"
        );
    }
}
