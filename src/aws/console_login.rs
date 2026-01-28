//! Console login support via AWS CLI subprocess
//!
//! This module provides functionality to spawn `aws login --profile <profile>` as a subprocess,
//! allowing taws to delegate the OAuth 2.0 PKCE flow to the AWS CLI.

use anyhow::Result;
use std::io::{BufRead, BufReader};
use std::process::{Child, Command, Stdio};
use std::sync::mpsc;
use std::thread;

/// Information extracted from aws login output
#[derive(Debug, Clone, Default)]
pub struct LoginInfo {
    pub url: Option<String>,
}

/// Check if AWS CLI supports `aws login` (v2.32.0+)
pub fn is_aws_login_available() -> bool {
    Command::new("aws")
        .args(["login", "help"])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .status()
        .map(|s| s.success())
        .unwrap_or(false)
}

/// Spawn `aws login --profile <profile>` subprocess
/// Returns the child process handle and a receiver for parsed login info (URL)
pub fn spawn_aws_login(profile: &str, region: &str) -> Result<(Child, mpsc::Receiver<LoginInfo>)> {
    let mut child = Command::new("aws")
        .args(["login", "--profile", profile, "--region", region])
        .stdout(Stdio::null()) // aws login outputs to stderr
        .stderr(Stdio::piped()) // capture for URL and error messages
        .spawn()?;

    let stderr = child.stderr.take().expect("stderr was piped");
    let (tx, rx) = mpsc::channel();

    // Spawn thread to read stderr and parse URL
    thread::spawn(move || {
        let reader = BufReader::new(stderr);
        let mut info = LoginInfo::default();

        for line in reader.lines().map_while(Result::ok) {
            // Parse URL from output (AWS CLI outputs URLs starting with https://)
            if line.contains("https://") {
                if let Some(url) = extract_url(&line) {
                    info.url = Some(url);
                    let _ = tx.send(info.clone());
                }
            }
        }
    });

    Ok((child, rx))
}

/// Extract URL from a line of text
fn extract_url(line: &str) -> Option<String> {
    let start = line.find("https://")?;
    let rest = &line[start..];
    // URL ends at whitespace or end of line
    let end = rest.find(char::is_whitespace).unwrap_or(rest.len());
    Some(rest[..end].to_string())
}

/// Check if the subprocess has completed
/// Returns: Ok(None) = still running, Ok(Some(true)) = success, Ok(Some(false)) = failed
pub fn check_login_status(child: &mut Child) -> Result<Option<bool>> {
    match child.try_wait()? {
        None => Ok(None), // still running
        Some(status) => Ok(Some(status.success())),
    }
}

/// Read stderr from a completed child process to get error message
pub fn read_child_stderr(child: &mut Child) -> Option<String> {
    use std::io::Read;
    child.stderr.take().and_then(|mut stderr| {
        let mut output = String::new();
        stderr.read_to_string(&mut output).ok()?;
        let trimmed = output.trim();
        if trimmed.is_empty() {
            None
        } else {
            Some(trimmed.to_string())
        }
    })
}
