use crate::{
    config::{Backend, Profile},
    error::Error,
};
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::Value;
use std::{path::PathBuf, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

pub const WRITE_SCRIPT: &str = include_str!("desktop_write.ps1");
pub const SCRIPT: &str = include_str!("desktop.ps1");
const MAX_OUTPUT: u64 = 32 * 1024 * 1024;

pub fn powershell() -> Result<PathBuf, Error> {
    let supported = cfg!(windows)
        || (cfg!(target_os = "linux")
            && std::fs::read_to_string("/proc/sys/kernel/osrelease")
                .unwrap_or_default()
                .to_ascii_lowercase()
                .contains("microsoft"));
    if !supported {
        return Err(Error::new(
            "unsupported",
            "Desktop access requires Windows or WSL with Windows interop, and the Windows OneNote desktop application. macOS and web OneNote do not expose this COM interface.",
        ));
    }
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|p| p.join("powershell.exe")).find(|p| p.is_file())
        .ok_or_else(|| Error::desktop("powershell.exe is not on PATH; enable Windows interop in WSL or restore Windows PowerShell to PATH"))
}

pub const REMOTE_SCRIPT: &str = include_str!("desktop_remote.ps1");

pub fn executable(profile: &Profile) -> Result<PathBuf, Error> {
    if profile.backend == Backend::Desktop {
        return powershell();
    }
    let name = if cfg!(windows) { "ssh.exe" } else { "ssh" };
    std::env::split_paths(&std::env::var_os("PATH").unwrap_or_default())
        .map(|p| p.join(name))
        .find(|p| p.is_file())
        .ok_or_else(|| {
            Error::desktop("OpenSSH client is not on PATH; install it before using an SSH profile")
        })
}
fn ssh_command(profile: &Profile) -> Result<Command, Error> {
    profile.validate()?;
    let mut command = Command::new(executable(profile)?);
    command.args([
        "-T",
        "-o",
        "BatchMode=yes",
        "-o",
        "ConnectTimeout=10",
        "-o",
        "ServerAliveInterval=10",
        "-o",
        "ServerAliveCountMax=2",
        "-o",
        "StrictHostKeyChecking=yes",
    ]);
    if let Some(path) = &profile.identity_file {
        command.arg("-i").arg(path);
    }
    if let Some(port) = profile.port {
        command.arg("-p").arg(port.to_string());
    }
    command
        .arg("--")
        .arg(profile.host.as_deref().unwrap_or_default())
        .arg("powershell.exe");
    Ok(command)
}
pub async fn run(profile: &Profile, request: Value) -> Result<String, Error> {
    let value = scan(profile, request).await?;
    value["xml"]
        .as_str()
        .map(str::to_owned)
        .ok_or_else(|| Error::desktop("OneNote bridge returned no XML"))
}
pub async fn scan(profile: &Profile, request: Value) -> Result<Value, Error> {
    execute(profile, request, SCRIPT).await
}
pub async fn write(profile: &Profile, request: Value) -> Result<Value, Error> {
    if profile.read_only {
        return Err(Error::new(
            "read_only",
            "This profile is read-only; select a writable profile to create or append pages",
        ));
    }
    execute(profile, request, WRITE_SCRIPT).await
}
async fn execute(profile: &Profile, request: Value, script: &str) -> Result<Value, Error> {
    if profile.backend == Backend::Desktop {
        run_bridge(
            Command::new(powershell()?),
            request,
            script,
            Duration::from_secs(45),
        )
        .await
    } else {
        run_bridge_inner(
            ssh_command(profile)?,
            request,
            script,
            Duration::from_secs(90),
            true,
        )
        .await.map_err(|mut error| {
            if error.message.contains("Invalid PowerShell response") {
                error.message.push_str(" Check SSH key/agent authentication, verify the host key with your SSH client, and confirm Windows PowerShell is available on the destination.");
            }
            error
        })
    }
}

async fn bounded(reader: impl AsyncRead + Unpin, max: u64) -> Result<Vec<u8>, Error> {
    let mut bytes = Vec::new();
    reader.take(max + 1).read_to_end(&mut bytes).await?;
    if bytes.len() as u64 > max {
        return Err(Error::desktop(
            "OneNote response exceeded the size limit; narrow the notebook or search scope",
        ));
    }
    Ok(bytes)
}

async fn run_bridge(
    command: Command,
    request: Value,
    script: &str,
    timeout: Duration,
) -> Result<Value, Error> {
    run_bridge_inner(command, request, script, timeout, false).await
}
async fn run_bridge_inner(
    mut command: Command,
    request: Value,
    script: &str,
    timeout: Duration,
    remote: bool,
) -> Result<Value, Error> {
    let executable_script = if remote { REMOTE_SCRIPT } else { script };
    let encoded = STANDARD.encode(
        executable_script
            .encode_utf16()
            .flat_map(u16::to_le_bytes)
            .collect::<Vec<_>>(),
    );
    let mut child = command
        .args([
            "-NoLogo",
            "-NoProfile",
            "-NonInteractive",
            "-STA",
            "-EncodedCommand",
            &encoded,
        ])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|e| {
            Error::desktop(format!(
                "Cannot start {}: {e}",
                if remote { "SSH" } else { "Windows PowerShell" }
            ))
        })?;
    let mut stdin = child
        .stdin
        .take()
        .ok_or_else(|| Error::desktop("Missing bridge stdin"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| Error::desktop("Missing bridge stdout"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| Error::desktop("Missing bridge stderr"))?;
    let payload = if remote {
        serde_json::json!({"script_base64":STANDARD.encode(format!("\u{feff}{script}")),"request":request,"write":script == WRITE_SCRIPT})
    } else {
        request.clone()
    };
    let bytes = serde_json::to_vec(&payload).map_err(|e| Error::invalid(e.to_string()))?;
    let uncertain = |e: Error| {
        if script == WRITE_SCRIPT {
            Error::new(
                "write_uncertain",
                format!(
                    "{}. The {} on target {} may have taken effect; inspect OneNote before retrying.",
                    e.message,
                    request["operation"].as_str().unwrap_or("write"),
                    request["target"].as_str().unwrap_or("unknown")
                ),
            )
        } else {
            e
        }
    };
    let operation = async {
        let (_, stdout, stderr, status) = tokio::try_join!(
            async {
                stdin.write_all(&bytes).await?;
                drop(stdin);
                Ok::<_, Error>(())
            },
            bounded(stdout, MAX_OUTPUT),
            bounded(stderr, 64 * 1024),
            async { Ok::<_, Error>(child.wait().await?) },
        )
        .map_err(&uncertain)?;
        // A valid error envelope is authoritative about whether mutation began.
        if script == WRITE_SCRIPT {
            let decoded: Option<Value> =
                serde_json::from_slice(stdout.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(&stdout))
                    .ok();
            if decoded.as_ref().is_some_and(|v| v.get("error").is_some()) {
                return parse_output(status.success(), &stdout, &stderr);
            }
            parse_output(status.success(), &stdout, &stderr).map_err(&uncertain)
        } else {
            parse_output(status.success(), &stdout, &stderr)
        }
    };
    let result = match tokio::time::timeout(timeout, operation).await {
        Ok(result) => result,
        Err(_) => Err(uncertain(Error::desktop(format!(
            "OneNote did not respond within {} seconds; check its Windows desktop for setup, password, or sync prompts",
            timeout.as_secs()
        )))),
    };
    if result.is_err() {
        let _ = child.kill().await;
    }
    result
}

fn parse_output(success: bool, stdout: &[u8], stderr: &[u8]) -> Result<Value, Error> {
    let value: Value =
        serde_json::from_slice(stdout.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(stdout))
            .map_err(|_| {
                Error::desktop(format!(
                    "Invalid PowerShell response: {}",
                    String::from_utf8_lossy(stderr).trim()
                ))
            })?;
    if let Some(error) = value.get("error") {
        let kind = match error["kind"].as_str() {
            Some("invalid_input") => "invalid_input",
            Some("not_found") => "not_found",
            Some("conflict") => "conflict",
            Some("write_uncertain") => "write_uncertain",
            _ => "desktop_error",
        };
        let mut failure = Error::new(
            kind,
            error["message"]
                .as_str()
                .unwrap_or("OneNote COM operation failed"),
        );
        failure.page_id = error["page_id"]
            .as_str()
            .filter(|id| !id.is_empty())
            .map(str::to_owned);
        return Err(failure);
    }
    if !success {
        return Err(Error::desktop(format!(
            "PowerShell failed: {}",
            String::from_utf8_lossy(stderr).trim()
        )));
    }
    Ok(value)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn script_fits_windows_command_line_and_protocol_errors_are_preserved() {
        for script in [SCRIPT, WRITE_SCRIPT, REMOTE_SCRIPT] {
            assert!(script.encode_utf16().count() * 2 * 4 / 3 < 32000);
        }
        assert!(parse_output(true, b"noise", b"").is_err());
        assert_eq!(
            parse_output(
                false,
                br#"{"error":{"kind":"not_found","message":"Gone"}}"#,
                b""
            )
            .unwrap_err()
            .kind,
            "not_found"
        );
        assert_eq!(
            parse_output(true, b"\xef\xbb\xbf{\"xml\":\"hello\"}\r\n", b"").unwrap()["xml"],
            "hello"
        );
    }
    #[tokio::test]
    async fn output_size_is_bounded() {
        assert!(bounded(&b"12345"[..], 4).await.is_err());
        assert_eq!(bounded(&b"1234"[..], 4).await.unwrap(), b"1234");
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn hung_bridge_times_out() {
        let mut command = Command::new("sh");
        command.args(["-c", "exec sleep 10"]);
        let error = run_bridge(
            command,
            serde_json::json!({}),
            SCRIPT,
            Duration::from_millis(50),
        )
        .await
        .unwrap_err();
        assert_eq!(error.kind, "desktop_error");
        assert!(error.message.contains("did not respond"));
    }
    #[cfg(unix)]
    #[tokio::test]
    async fn write_transport_failures_are_uncertain_but_com_errors_are_preserved() {
        for (shell, expected) in [
            ("exec sleep 10", "write_uncertain"),
            ("cat >/dev/null; printf noise", "write_uncertain"),
            (
                "cat >/dev/null; printf '%s' '{\"error\":{\"kind\":\"desktop_error\",\"message\":\"Locked\",\"page_id\":\"page\"}}'; exit 1",
                "desktop_error",
            ),
        ] {
            let mut command = Command::new("sh");
            command.args(["-c", shell]);
            let error = run_bridge(
                command,
                serde_json::json!({"operation":"append","target":"page"}),
                WRITE_SCRIPT,
                Duration::from_millis(100),
            )
            .await
            .unwrap_err();
            assert_eq!(error.kind, expected);
            if expected == "desktop_error" {
                assert_eq!(error.page_id.as_deref(), Some("page"));
            } else {
                assert!(error.message.contains("inspect OneNote before retrying"));
            }
        }
    }
}
