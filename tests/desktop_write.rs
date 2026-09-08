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
fn production_write_bridge_preserves_objects_and_reports_uncertain_outcomes() {
    let mock = r#"
function New-MockApplication {
    $app = New-Object PSObject -Property @{reads=0;created=$false;updated=$false;state=$null}
    $app.state = [xml]'<one:Page xmlns:one="http://schemas.microsoft.com/office/onenote/2013/onenote" ID="page" name="Existing" lastModifiedTime="2026-09-08T12:00:00.000Z"><one:Title><one:OE><one:T>Existing</one:T></one:OE></one:Title><one:Outline objectID="old"><one:Position x="36" y="100"/><one:Size height="80"/><one:OEChildren><one:OE><one:T>Existing body &amp;amp; content</one:T></one:OE></one:OEChildren></one:Outline><one:Image objectID="image"><one:Position x="36" y="200"/><one:Size height="300"/></one:Image><one:InsertedFile objectID="file"/></one:Page>'
    $app | Add-Member ScriptMethod GetHierarchy {
        param($target,$depth,$xml,$schema)
        if ($target -ne 'section' -or $depth -ne 3 -or $schema -ne 2) {throw 'Wrong section preflight'}
        $xml.Value='<one:Section xmlns:one="http://schemas.microsoft.com/office/onenote/2013/onenote" ID="section"/>'
    }
    $app | Add-Member ScriptMethod CreateNewPage {
        param($section,$id,$style)
        if ($style -ne 1 -or $section -ne 'section') {throw 'Wrong page creation'}
        $this.created=$true
        $id.Value='page'
    }
    $app | Add-Member ScriptMethod GetPageContent {
        param($id,$xml,$info,$schema)
        if ($id -ne 'page' -or $info -ne 0 -or $schema -ne 2) {throw 'Unexpected read'}
        if ($request.test_mode -eq 'locked') {throw [Runtime.InteropServices.COMException]::new('Locked', -2147213285)}
        if ($this.updated -and $request.test_mode -eq 'read-fail') {throw 'Read-back unavailable'}
        $this.reads++
        $xml.Value=$this.state.OuterXml
    }
    $app | Add-Member ScriptMethod UpdatePageContent {
        param($xml,$expected,$schema,$force)
        if ($schema -ne 2 -or $force -or $expected.ToString('yyyy-MM-ddTHH:mm:ss') -ne '2026-09-08T12:00:00') {throw 'Unchecked write'}
        if ($request.test_mode -eq 'conflict') {throw [Runtime.InteropServices.COMException]::new('Changed', -2147213296)}
        if ($request.test_mode -eq 'update-fail') {throw 'Update failed'}
        $changes=[xml]$xml
        $page=$changes.DocumentElement
        if ($page.ID -ne 'page' -or @($page.ChildNodes | Where-Object LocalName -notin @('Title','Outline')).Count -ne 0) {throw 'Unexpected page-level replacement'}
        $outline=@($page.ChildNodes | Where-Object LocalName -eq 'Outline')
        if ($outline.Count -ne 1 -or $outline[0].HasAttribute('objectID') -or [double]$outline[0].Position.y -lt 524) {throw 'Existing outline replacement or overlapping content'}
        if (@($outline[0].OEChildren.OE).Count -ne 2) {throw 'Input lines must use separate OneNote elements'}
        $titles=@($page.ChildNodes | Where-Object LocalName -eq 'Title')
        if (-not $this.created -and $titles.Count -ne 0) {throw 'Append replaced title'}
        if ($this.created) {
            $oldTitle=$this.state.DocumentElement.FirstChild
            [void]$this.state.DocumentElement.ReplaceChild($this.state.ImportNode($titles[0],$true),$oldTitle)
            $this.state.DocumentElement.SetAttribute('name',[string]$request.title)
        }
        if ($request.test_mode -ne 'missing-block') {
            $new=$this.state.ImportNode($outline[0],$true)
            $new.SetAttribute('objectID','new')
            [void]$this.state.DocumentElement.AppendChild($new)
        }
        if ($request.test_mode -eq 'drop-old') {
            [void]$this.state.DocumentElement.RemoveChild($this.state.DocumentElement.SelectSingleNode('*[@objectID="image"]'))
        }
        $this.updated=$true
    }
    return $app
}
"#;
    let script = format!(
        "\u{feff}{mock}\n{}",
        onenote_cli::desktop::WRITE_SCRIPT.replace(
            "New-Object -ComObject OneNote.Application",
            "New-MockApplication"
        )
    );
    let dir = tempfile::tempdir().unwrap();
    let path = dir.path().join("write.ps1");
    std::fs::write(&path, script).unwrap();
    for (operation, mode, kind) in [
        ("create", "ok", ""),
        ("append", "ok", ""),
        ("append", "conflict", "conflict"),
        ("create", "conflict", "write_uncertain"),
        ("create", "update-fail", "write_uncertain"),
        ("append", "read-fail", "write_uncertain"),
        ("append", "missing-block", "write_uncertain"),
        ("append", "drop-old", "write_uncertain"),
        ("append", "locked", "desktop_error"),
    ] {
        let input = json!({"operation":operation,"target":if operation == "create" {"section"} else {"page"},"title":"Title <&> 日本語 ]]> $(exit)","text":"Literal <script> & 日本語 ]]> $(exit)\nSecond line","test_mode":mode});
        let mut cmd = Command::new(
            std::env::var_os("ONENOTE_TEST_POWERSHELL").unwrap_or_else(|| "powershell.exe".into()),
        );
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
            .write_all(&serde_json::to_vec(&input).unwrap())
            .unwrap();
        let output = child.wait_with_output().unwrap();
        let text = String::from_utf8_lossy(&output.stdout);
        let result: Value = serde_json::from_str(text.trim_start_matches('\u{feff}'))
            .unwrap_or_else(|e| panic!("{e}: {text} {}", String::from_utf8_lossy(&output.stderr)));
        assert_eq!(
            output.status.success(),
            kind.is_empty(),
            "{operation} {mode}: {result}"
        );
        if kind.is_empty() {
            assert_eq!(result["verified"], true);
            let xml = result["xml"].as_str().unwrap();
            assert!(xml.contains("objectID=\"old\""));
            assert!(xml.contains("objectID=\"image\""));
            assert!(xml.contains("objectID=\"file\""));
            let page = onenote_cli::model::read(xml, "page", false).unwrap();
            assert!(page["markdown"].as_str().unwrap().contains("日本語"));
        } else {
            assert_eq!(
                result["error"]["kind"], kind,
                "{operation} {mode}: {result}"
            );
            assert_eq!(result["error"]["page_id"], "page");
        }
    }
}
