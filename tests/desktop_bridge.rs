//! Run the production script with only COM creation replaced by a mock.
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
fn production_bridge_uses_read_only_com_methods_with_literal_arguments() {
    let mock = r#"
function Get-Service {
    param($Name)
    if ($request.query -eq 'service-stopped') { return @{Status='Stopped'} }
    return @{Status='Running'}
}
function New-MockApplication {
    $app = New-Object PSObject
    $app | Add-Member ScriptMethod GetHierarchy {
        param($scope,$depth,$xml,$schema)
        if ($schema -ne 2) { throw 'Wrong schema' }
        $xml.Value = "hierarchy:$scope/$depth"
    }
    $app | Add-Member ScriptMethod GetPageContent {
        param($id,$xml,$info,$schema)
        if ($schema -ne 2 -or $info -ne 0) { throw 'Unexpected binary request' }
        if ($id -eq 'first-run') { throw [Runtime.InteropServices.COMException]::new('Unspecified error', -2147467259) }
        if ($id -eq 'locked') { throw [Runtime.InteropServices.COMException]::new('Section encrypted', -2147213285) }
        $xml.Value = "page:$id"
    }
    $app | Add-Member ScriptMethod FindPages {
        param($scope,$query,$xml,$unindexed,$display,$schema)
        if ($schema -ne 2 -or $display -or -not $unindexed) { throw 'Unexpected search options' }
        $xml.Value = "search:$scope/$query"
    }
    return $app
}
"#;
    let script = format!(
        "\u{feff}{mock}\n{}",
        onenote_cli::desktop::SCRIPT.replace(
            "New-Object -ComObject OneNote.Application",
            "New-MockApplication"
        )
    );
    let temp = tempfile::tempdir().unwrap();
    let path = temp.path().join("bridge.ps1");
    std::fs::write(&path, script).unwrap();
    for (request, expected) in [
        (json!({"operation":"probe"}), "hierarchy:/2"),
        (json!({"operation":"notebooks"}), "hierarchy:/2"),
        (
            json!({"operation":"sections","scope":"notebook"}),
            "hierarchy:notebook/3",
        ),
        (
            json!({"operation":"pages","scope":"section"}),
            "hierarchy:section/4",
        ),
        (
            json!({"operation":"read","id":"O'Brien; $(exit); 日本語"}),
            "page:O'Brien; $(exit); 日本語",
        ),
        (
            json!({"operation":"search","scope":"section","query":"O'Brien; $(exit); 日本語"}),
            "search:section/O'Brien; $(exit); 日本語",
        ),
        (json!({"operation":"delete"}), "error"),
        (json!({"operation":"read","id":"locked"}), "locked-error"),
        (
            json!({"operation":"search","query":"service-stopped"}),
            "search-service-error",
        ),
        (
            json!({"operation":"read","id":"first-run"}),
            "first-run-error",
        ),
    ] {
        let executable =
            std::env::var_os("ONENOTE_TEST_POWERSHELL").unwrap_or_else(|| "powershell.exe".into());
        let mut cmd = Command::new(executable);
        cmd.args(["-NoLogo", "-NoProfile", "-NonInteractive"]);
        if cfg!(windows) {
            cmd.args(["-STA", "-ExecutionPolicy", "Bypass"]);
        }
        let mut child = cmd
            .arg("-File")
            .arg(&path)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .unwrap();
        child
            .stdin
            .take()
            .unwrap()
            .write_all(&serde_json::to_vec(&request).unwrap())
            .unwrap();
        let result = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&result.stdout);
        let value: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .unwrap_or_else(|e| panic!("{e}: {text}, {}", String::from_utf8_lossy(&result.stderr)));
        assert_eq!(
            result.status.success(),
            !expected.ends_with("error"),
            "{value}"
        );
        if expected == "error" {
            assert_eq!(value["error"]["kind"], "invalid_input");
        } else if expected == "locked-error" {
            assert_eq!(value["error"]["kind"], "desktop_error");
            assert!(
                value["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("Unlock it in OneNote desktop")
            );
        } else if expected == "first-run-error" {
            assert_eq!(value["error"]["kind"], "desktop_error");
            assert!(
                value["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("first-run sign-in")
            );
        } else if expected == "search-service-error" {
            assert_eq!(value["error"]["kind"], "desktop_error");
            assert!(
                value["error"]["message"]
                    .as_str()
                    .unwrap()
                    .contains("Windows Search")
            );
        } else {
            assert_eq!(value["xml"], expected);
        }
    }
}
