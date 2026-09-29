//! Exercise task lifecycle and error classification with Windows APIs mocked.
use base64::{Engine, engine::general_purpose::STANDARD};
use serde_json::{Value, json};
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
#[cfg_attr(
    not(windows),
    ignore = "requires PowerShell; set ONENOTE_TEST_POWERSHELL and run --ignored"
)]
fn remote_task_cleans_up_and_preserves_write_outcomes() {
    let temp = tempfile::tempdir().unwrap();
    let original = onenote_cli::desktop::REMOTE_SCRIPT;
    let acl_start = original.find("    $user = ").unwrap();
    let acl_end = original.find("    $utf8 = ").unwrap();
    let mut script = original.to_string();
    script.replace_range(
        acl_start..acl_end,
        r#"
    $user = @{Name='test-user'}
    $directory = Join-Path $env:ONENOTE_TEST_ROOT $task
    [void][IO.Directory]::CreateDirectory($directory)
"#,
    );
    script = script.replace(
        "$clock.Elapsed.TotalSeconds -gt 65",
        "$clock.Elapsed.TotalSeconds -gt 0.1",
    );
    let mock = r#"
function New-ScheduledTaskAction { param($Execute,$Argument,$WorkingDirectory) return @{Args=$Argument;Directory=$WorkingDirectory} }
function New-ScheduledTaskPrincipal {
    param($UserId,$LogonType,$RunLevel)
    if ($LogonType -ne 'Interactive' -or $RunLevel -ne 'Limited') { throw 'Incorrect desktop principal' }
    return @{}
}
function New-ScheduledTaskSettingsSet { param($ExecutionTimeLimit,[switch]$AllowStartIfOnBatteries,[switch]$DontStopIfGoingOnBatteries) return @{} }
function Register-ScheduledTask {
    param($TaskName,$Action,$Principal,$Settings)
    if ($envelope.request.mode -eq 'register-fail') { throw 'Registration refused' }
    [IO.File]::WriteAllText((Join-Path $env:ONENOTE_TEST_ROOT 'registered'), $TaskName)
}
function Start-ScheduledTask {
    param($TaskName)
    $input = [IO.File]::ReadAllText((Join-Path $directory 'input.json')) | ConvertFrom-Json
    if ($input.text -ne 'Literal $(exit) 日本語') { throw 'Request changed' }
    if ([IO.File]::ReadAllText((Join-Path $directory 'bridge.ps1')) -ne '# trusted bridge') { throw 'Script changed' }
    switch ($input.mode) {
        'start-fail' { throw 'Start interrupted' }
        'timeout' { return }
        'runner-fail' { [IO.File]::WriteAllText((Join-Path $directory 'failed'),'Runner failed'); return }
        'empty' { $response = '' }
        'bridge-error' { $response = '{"error":{"kind":"not_found","message":"Gone"}}' }
        default { $response = '{"xml":"日本語"}' }
    }
    [IO.File]::WriteAllText((Join-Path $directory 'output.json'), $response)
    [IO.File]::WriteAllText((Join-Path $directory 'done'), 'complete')
}
function Stop-ScheduledTask { param($TaskName,$ErrorAction) }
function Unregister-ScheduledTask {
    param($TaskName,$Confirm,$ErrorAction)
    Remove-Item (Join-Path $env:ONENOTE_TEST_ROOT 'registered')
}
"#;
    let path = temp.path().join("remote.ps1");
    std::fs::write(&path, format!("\u{feff}{mock}\n{script}")).unwrap();
    let executable =
        std::env::var("ONENOTE_TEST_POWERSHELL").unwrap_or_else(|_| "powershell.exe".into());
    for (mode, write, error) in [
        ("ok", false, None),
        ("ok", true, None),
        ("register-fail", true, Some("desktop_error")),
        ("start-fail", true, Some("write_uncertain")),
        ("timeout", true, Some("write_uncertain")),
        ("timeout", false, Some("desktop_error")),
        ("runner-fail", true, Some("write_uncertain")),
        ("empty", true, Some("write_uncertain")),
        ("bridge-error", true, Some("not_found")),
    ] {
        let request = json!({"script_base64": STANDARD.encode("# trusted bridge"),"request":{"mode":mode,"text":"Literal $(exit) 日本語"},"write":write});
        let mut child = Command::new(&executable)
            .args(["-NoLogo", "-NoProfile", "-NonInteractive", "-File"])
            .arg(&path)
            .env("ONENOTE_TEST_ROOT", temp.path())
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(request.to_string().as_bytes())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        assert!(
            output.status.success(),
            "{}",
            String::from_utf8_lossy(&output.stderr)
        );
        let value: Value = serde_json::from_slice(
            output
                .stdout
                .strip_prefix(&[0xef, 0xbb, 0xbf])
                .unwrap_or(&output.stdout),
        )
        .unwrap();
        assert_eq!(
            value["error"]["kind"].as_str(),
            error,
            "mode {mode}: {value}"
        );
        if error.is_none() {
            assert_eq!(value["xml"], "日本語");
        }
        assert!(!temp.path().join("registered").exists());
        assert_eq!(
            std::fs::read_dir(temp.path()).unwrap().count(),
            1,
            "request files leaked after {mode}"
        );
    }
}
