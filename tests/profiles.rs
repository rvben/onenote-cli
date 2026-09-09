use assert_cmd::Command;
use onenote_cli::config::{Backend, Profile, Store};
use serde_json::Value;

struct Fixture {
    directory: tempfile::TempDir,
}
impl Fixture {
    fn new() -> Self {
        Self {
            directory: tempfile::tempdir().unwrap(),
        }
    }
    fn store(&self) -> Store {
        Store {
            path: self.directory.path().join("config.toml"),
        }
    }
    fn command(&self) -> Command {
        let mut cmd = Command::new(assert_cmd::cargo::cargo_bin!("onenote"));
        cmd.env("ONENOTE_CONFIG", self.store().path)
            .env_remove("ONENOTE_PROFILE");
        cmd
    }
    fn run(&self, args: &[&str]) -> Value {
        let result = self.command().args(args).assert().success();
        serde_json::from_slice(&result.get_output().stdout).unwrap()
    }
}
#[test]
fn setup_selection_precedence_replacement_and_removal() {
    let f = Fixture::new();
    assert_eq!(f.run(&["config", "show"])["source"], "implicit_desktop");
    assert_eq!(
        f.run(&["profile", "list"])["profiles"],
        serde_json::json!([])
    );
    let first = f.run(&["init", "--profile", "local", "--no-check"]);
    assert_eq!(first["verified"], false);
    let before = std::fs::read(f.store().path).unwrap();
    f.command()
        .args(["init", "--profile", "local", "--no-check", "--read-only"])
        .assert()
        .code(2);
    assert_eq!(std::fs::read(f.store().path).unwrap(), before);
    f.run(&[
        "init",
        "--profile",
        "remote",
        "--backend",
        "ssh",
        "--host",
        "kiosk",
        "--read-only",
        "--no-check",
    ]);
    assert_eq!(f.run(&["config", "show"])["profile"], "remote");
    let result = f
        .command()
        .env("ONENOTE_PROFILE", "remote")
        .args(["--profile", "local", "config", "show"])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert_eq!(value["profile"], "local");
    f.run(&["profile", "use", "local"]);
    assert_eq!(f.run(&["config", "show"])["profile"], "local");
    let result = f
        .command()
        .env("ONENOTE_PROFILE", "remote")
        .args(["config", "show"])
        .assert()
        .success();
    let value: Value = serde_json::from_slice(&result.get_output().stdout).unwrap();
    assert_eq!(value["profile"], "remote");
    f.run(&[
        "init",
        "--profile",
        "local",
        "--no-check",
        "--read-only",
        "--force",
    ]);
    assert_eq!(f.run(&["config", "show"])["connection"]["read_only"], true);
    f.run(&["profile", "remove", "local"]);
    // Never silently switch to another machine after removing the active profile.
    f.command().args(["doctor", "--offline"]).assert().code(2);
    f.run(&["profile", "use", "remote"]);
    assert_eq!(f.run(&["config", "show"])["profile"], "remote");
}
#[test]
fn read_only_refuses_writes_before_launch_but_allows_preview() {
    let f = Fixture::new();
    f.run(&[
        "init",
        "--no-check",
        "--backend",
        "ssh",
        "--host",
        "unreachable.invalid",
        "--read-only",
    ]);
    let result = f
        .command()
        .env("PATH", "")
        .args(["pages", "append", "page", "--text", "日本語"])
        .assert()
        .code(2);
    let error: Value = serde_json::from_slice(&result.get_output().stderr).unwrap();
    assert_eq!(error["error"]["kind"], "read_only");
    let result = f.run(&["pages", "append", "page", "--text", "日本語", "--dry-run"]);
    assert_eq!(result["connection"]["host"], "unreachable.invalid");
    assert_eq!(result["connection"]["read_only"], true);
    assert_eq!(result["dry_run"], true);
}
#[test]
fn malformed_configuration_is_not_overwritten_and_offline_discovery_still_works() {
    let f = Fixture::new();
    for content in [
        "oops",
        "version = 2",
        "version = 1\nunknown = true",
        "active_profile = 'missing'",
        "[profiles.bad]\nbackend = 'ssh'\nhost = '-oProxyCommand=bad'",
    ] {
        std::fs::write(f.store().path, content).unwrap();
        f.command().args(["config", "show"]).assert().code(2);
        f.command()
            .args(["init", "--no-check", "--force"])
            .assert()
            .code(2);
        assert_eq!(std::fs::read_to_string(f.store().path).unwrap(), content);
        for args in [vec!["config", "path"], vec!["schema"], vec!["capabilities"]] {
            f.run(&args);
        }
    }
}
#[test]
fn validates_destinations_and_profile_names_and_key_paths() {
    let f = Fixture::new();
    for host in [
        "",
        "-evil",
        "host;exit",
        "host$(id)",
        "host name",
        "host\nname",
        "@host",
        "user@",
        "a@b@c",
    ] {
        assert!(
            Profile {
                backend: Backend::Ssh,
                host: Some(host.into()),
                ..Profile::default()
            }
            .validate()
            .is_err()
        );
    }
    for host in ["kiosk", "user@kiosk", "100.99.48.87", "user@[::1]", "::1"] {
        assert!(
            Profile {
                backend: Backend::Ssh,
                host: Some(host.into()),
                ..Profile::default()
            }
            .validate()
            .is_ok()
        );
    }
    for name in ["", "a b", "../escape", "日本語", "bad\x1b"] {
        f.command()
            .args(["init", "--profile", name, "--no-check"])
            .assert()
            .code(2);
    }
    for args in [
        vec!["init", "--host", "kiosk", "--no-check"],
        vec!["init", "--backend", "ssh", "--no-check"],
        vec![
            "init",
            "--backend",
            "ssh",
            "--host",
            "kiosk",
            "--identity-file",
            "/no/such/key",
            "--no-check",
        ],
    ] {
        f.command().args(args).assert().code(2);
    }
    assert!(!f.store().path.exists());
    let key = f.directory.path().join("key with spaces");
    std::fs::write(&key, "test key contents that must never be copied").unwrap();
    f.run(&[
        "init",
        "--backend",
        "ssh",
        "--host",
        "kiosk",
        "--identity-file",
        key.to_str().unwrap(),
        "--no-check",
    ]);
    let config = std::fs::read_to_string(f.store().path).unwrap();
    assert!(!config.contains("test key contents"));
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        assert_eq!(
            std::fs::metadata(f.store().path)
                .unwrap()
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
    }
}
#[test]
fn concurrent_config_update_fails_without_losing_settings() {
    let f = Fixture::new();
    f.run(&["init", "--no-check"]);
    let lock = std::fs::OpenOptions::new()
        .read(true)
        .write(true)
        .open(f.store().path.with_extension("toml.lock"))
        .unwrap();
    lock.lock().unwrap();
    f.command()
        .args(["profile", "remove", "default"])
        .assert()
        .code(2);
    assert!(
        f.store()
            .read()
            .unwrap()
            .unwrap()
            .profiles
            .contains_key("default")
    );
    drop(lock);
    f.run(&["profile", "remove", "default"]);
}
#[test]
fn text_setup_has_clear_next_steps_and_no_json_dump() {
    let f = Fixture::new();
    let result = f
        .command()
        .args(["init", "--no-check", "-o", "text", "--no-color"])
        .assert()
        .success();
    let text = String::from_utf8_lossy(&result.get_output().stdout);
    assert!(text.contains("OneNote · default"));
    assert!(text.contains("Next: onenote doctor"));
    assert!(text.contains("Connection not checked"));
    assert!(!text.contains('{'));
    assert!(!text.contains('\x1b'));
}

#[test]
#[cfg(unix)]
fn ssh_keeps_requests_out_of_arguments_and_failed_checks_do_not_save() {
    use base64::{Engine, engine::general_purpose::STANDARD};
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new();
    let fake = f.directory.path().join("ssh");
    std::fs::write(&fake, "#!/bin/sh\nprintf '%s\\n' \"$@\" > \"$ONENOTE_TEST_ARGS\"\n/bin/cat > \"$ONENOTE_TEST_INPUT\"\nprintf '%s' '{\"error\":{\"kind\":\"desktop_error\",\"message\":\"Test connection refused\"}}'\nexit 1\n").unwrap();
    std::fs::set_permissions(&fake, std::fs::Permissions::from_mode(0o700)).unwrap();
    let argv = f.directory.path().join("args");
    let input = f.directory.path().join("input");
    let key = f.directory.path().join("key with spaces");
    std::fs::write(&key, "fake key").unwrap();
    let mut command = f.command();
    command
        .env("PATH", f.directory.path())
        .env("ONENOTE_TEST_ARGS", &argv)
        .env("ONENOTE_TEST_INPUT", &input);
    command
        .args([
            "init",
            "--profile",
            "remote",
            "--backend",
            "ssh",
            "--host",
            "test-user@kiosk",
            "--port",
            "2222",
            "--identity-file",
        ])
        .arg(&key)
        .assert()
        .code(5);
    assert!(!f.store().path.exists(), "failed init persisted settings");
    let args = std::fs::read_to_string(&argv).unwrap();
    let args: Vec<_> = args.lines().collect();
    assert!(args.windows(2).any(|w| w == ["-o", "BatchMode=yes"]));
    assert!(
        args.windows(2)
            .any(|w| w == ["-o", "StrictHostKeyChecking=yes"])
    );
    assert!(
        args.windows(3)
            .any(|w| w == ["--", "test-user@kiosk", "powershell.exe"])
    );
    assert!(
        args.windows(2)
            .any(|w| w == ["-i", key.canonicalize().unwrap().to_str().unwrap()])
    );
    let bytes = STANDARD.decode(args.last().unwrap()).unwrap();
    let units: Vec<_> = bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| u16::from_le_bytes([c[0], c[1]]))
        .collect();
    assert_eq!(
        String::from_utf16(&units).unwrap(),
        onenote_cli::desktop::REMOTE_SCRIPT
    );
    let payload: Value = serde_json::from_slice(&std::fs::read(&input).unwrap()).unwrap();
    assert_eq!(payload["request"]["operation"], "probe");
    assert_eq!(payload["write"], false);
    f.run(&["init", "--backend", "ssh", "--host", "kiosk", "--no-check"]);
    let content = "unique user payload $(exit) 日本語 <&>";
    f.command()
        .env("PATH", f.directory.path())
        .env("ONENOTE_TEST_ARGS", &argv)
        .env("ONENOTE_TEST_INPUT", &input)
        .args(["pages", "append", "page", "--text", content])
        .assert()
        .code(5);
    assert!(!std::fs::read_to_string(&argv).unwrap().contains(content));
    let payload: Value = serde_json::from_slice(&std::fs::read(&input).unwrap()).unwrap();
    assert_eq!(payload["request"]["text"], content);
    assert_eq!(payload["write"], true);
    assert_eq!(
        String::from_utf8(
            STANDARD
                .decode(payload["script_base64"].as_str().unwrap())
                .unwrap()
        )
        .unwrap(),
        format!("\u{feff}{}", onenote_cli::desktop::WRITE_SCRIPT)
    );
}
