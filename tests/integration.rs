use std::path::PathBuf;
use std::process::Command;
use std::time::Duration;

fn clamor_bin() -> PathBuf {
    env!("CARGO_BIN_EXE_clamor").into()
}

fn clamor_cmd() -> Command {
    let mut cmd = Command::new(clamor_bin());
    cmd.env("CLAMOR_DEBUG", "1");
    cmd.env_remove("XDG_CONFIG_HOME");
    cmd
}

/// Create an isolated HOME directory with a minimal clamor YAML config.
fn setup_test_env() -> PathBuf {
    let test_home =
        std::env::temp_dir().join(format!("clm-t-{}-{}", std::process::id(), rand_suffix()));
    std::fs::create_dir_all(test_home.join(".clamor")).unwrap();
    std::fs::create_dir_all(test_home.join(".config/clamor")).unwrap();

    let config_yaml = format!(
        r#"backends:
  claude-code:
    display_name: Claude
    spawn:
      cmd: [claude, "{{{{prompt}}}}"]
    resume:
      cmd: [claude, --resume, "{{{{resume_token}}}}"]
    capabilities:
      resume: true
folders:
  test:
    path: {path}
    backends: [claude-code]
"#,
        path = test_home.to_string_lossy()
    );

    std::fs::write(test_home.join(".config/clamor/config.yaml"), config_yaml).unwrap();

    test_home
}

fn rand_suffix() -> u64 {
    use std::sync::atomic::{AtomicU64, Ordering};
    use std::time::SystemTime;
    static COUNTER: AtomicU64 = AtomicU64::new(0);
    let count = COUNTER.fetch_add(1, Ordering::Relaxed);
    let nanos = SystemTime::now()
        .duration_since(SystemTime::UNIX_EPOCH)
        .unwrap()
        .as_nanos() as u64;
    nanos.wrapping_add(count)
}

/// Kill any daemon and remove the temp directory.
fn cleanup_test_env(test_home: &PathBuf) {
    let pid_file = test_home.join(".clamor/clamor.pid");
    if let Ok(pid_str) = std::fs::read_to_string(&pid_file) {
        if let Ok(pid) = pid_str.trim().parse::<i32>() {
            unsafe {
                libc::kill(pid, libc::SIGTERM);
            }
            // Brief wait for process to exit
            std::thread::sleep(Duration::from_millis(100));
        }
    }
    let _ = std::fs::remove_dir_all(test_home);
}

/// Start the daemon in the background and wait for it to be ready.
fn start_daemon(home: &PathBuf) {
    #[allow(clippy::zombie_processes)]
    let mut child = Command::new(clamor_bin())
        .arg("daemon")
        .env("HOME", home)
        .env("CLAMOR_DEBUG", "1")
        .env_remove("XDG_CONFIG_HOME")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to start daemon");

    // Wait for the socket to appear
    let sock = home.join(".clamor/clamor.sock");
    for _ in 0..50 {
        if sock.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    // If we get here, the daemon didn't start
    let _ = child.kill();
    panic!("daemon did not start within 5s");
}

#[test]
fn test_daemon_starts_and_stops() {
    let home = setup_test_env();
    start_daemon(&home);

    assert!(home.join(".clamor/clamor.pid").exists());
    assert!(home.join(".clamor/clamor.sock").exists());

    cleanup_test_env(&home);
}

#[test]
fn test_spawn_and_list() {
    let home = setup_test_env();
    start_daemon(&home);

    // Spawn an agent
    let output = clamor_cmd()
        .args(["new", "--folder", "test", "test task"])
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "spawn failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // List agents
    let output = clamor_cmd().arg("ls").env("HOME", &home).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("test task"), "agent not listed: {stdout}");
    assert!(
        stdout.contains("work"),
        "agent not in working state: {stdout}"
    );

    cleanup_test_env(&home);
}

#[test]
fn test_spawn_and_kill() {
    let home = setup_test_env();
    start_daemon(&home);

    // Spawn
    let output = clamor_cmd()
        .args(["new", "--folder", "test", "kill me"])
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(output.status.success());

    // Extract agent ID from output ("Spawned agent XXXXXX: kill me")
    let stdout = String::from_utf8_lossy(&output.stdout);
    let id = stdout
        .split_whitespace()
        .nth(2)
        .unwrap()
        .trim_end_matches(':');

    // Kill it
    let output = clamor_cmd()
        .args(["kill", id])
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "kill failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    // Verify it's gone from list
    let output = clamor_cmd().arg("ls").env("HOME", &home).output().unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        !stdout.contains("kill me") || stdout.contains("No agents"),
        "agent still listed: {stdout}"
    );

    cleanup_test_env(&home);
}

#[test]
fn test_kill_all() {
    let home = setup_test_env();
    start_daemon(&home);

    // Spawn two agents
    for desc in ["agent one", "agent two"] {
        let output = clamor_cmd()
            .args(["new", "--folder", "test", desc])
            .env("HOME", &home)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "spawn '{}' failed: {}",
            desc,
            String::from_utf8_lossy(&output.stderr)
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    // Kill all
    let output = clamor_cmd()
        .args(["kill", "--all"])
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "kill --all failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert!(String::from_utf8_lossy(&output.stdout).contains("2 agent(s)"));

    // List should be empty
    let output = clamor_cmd().arg("ls").env("HOME", &home).output().unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("No agents"));

    cleanup_test_env(&home);
}

#[test]
fn test_adopt_session() {
    let home = setup_test_env();
    start_daemon(&home);

    let output = clamor_cmd()
        .args([
            "adopt",
            "fake-session-123",
            "--folder",
            "test",
            "-d",
            "adopted task",
        ])
        .env("HOME", &home)
        .output()
        .unwrap();

    assert!(
        output.status.success(),
        "adopt failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(
        stdout.contains("Adopted session fake-session-123"),
        "unexpected output: {stdout}"
    );

    // Should appear in list
    let output = clamor_cmd().arg("ls").env("HOME", &home).output().unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("adopted task"));

    cleanup_test_env(&home);
}

#[test]
fn test_persistent_keys() {
    let home = setup_test_env();
    start_daemon(&home);

    // Spawn three agents
    for desc in ["first", "second", "third"] {
        let output = clamor_cmd()
            .args(["new", "--folder", "test", desc])
            .env("HOME", &home)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "spawn '{}' failed: {}",
            desc,
            String::from_utf8_lossy(&output.stderr)
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    // Read state file to verify keys
    let state_path = home.join(".clamor/state.json");
    let state_str = std::fs::read_to_string(&state_path).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state_str).unwrap();

    let agents = state["agents"].as_object().unwrap();
    let keys: Vec<Option<&str>> = agents.values().map(|a| a["key"].as_str()).collect();

    // All agents should have keys assigned
    assert!(
        keys.iter().all(|k| k.is_some()),
        "not all agents have keys: {keys:?}"
    );

    // Keys should be unique
    let key_set: std::collections::HashSet<&str> = keys.iter().filter_map(|k| *k).collect();
    assert_eq!(key_set.len(), 3, "keys are not unique: {keys:?}");

    // Keys should be from the pool
    let pool = ['a', 's', 'd', 'f', 'j', 'k', 'l', 'g', 'h'];
    for key in &key_set {
        assert!(
            pool.contains(&key.chars().next().unwrap()),
            "key '{key}' not in pool"
        );
    }

    cleanup_test_env(&home);
}

#[test]
fn test_color_indices_increment() {
    let home = setup_test_env();
    start_daemon(&home);

    for desc in ["a", "b", "c"] {
        let output = clamor_cmd()
            .args(["new", "--folder", "test", desc])
            .env("HOME", &home)
            .output()
            .unwrap();
        assert!(
            output.status.success(),
            "spawn '{}' failed: {}",
            desc,
            String::from_utf8_lossy(&output.stderr)
        );
        std::thread::sleep(Duration::from_millis(100));
    }

    let state_str = std::fs::read_to_string(home.join(".clamor/state.json")).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state_str).unwrap();

    let agents = state["agents"].as_object().unwrap();
    let mut colors: Vec<u64> = agents
        .values()
        .map(|a| a["color_index"].as_u64().unwrap())
        .collect();
    colors.sort();

    assert_eq!(colors, vec![0, 1, 2], "colors not sequential: {colors:?}");

    cleanup_test_env(&home);
}

#[test]
fn test_mock_agent_produces_output() {
    let output = Command::new(clamor_bin())
        .args(["mock-agent", "--description", "test", "--duration", "3"])
        .env("CLAMOR_AGENT_ID", "test123")
        .output()
        .unwrap();

    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Mock Agent"), "missing banner: {stdout}");
    assert!(stdout.contains("test123"), "missing agent ID: {stdout}");
    assert!(stdout.contains("tick"), "missing tick output: {stdout}");
    assert!(stdout.contains("done"), "missing completion: {stdout}");
}

#[test]
fn test_clean_removes_done_agents() {
    let home = setup_test_env();
    start_daemon(&home);

    // Spawn an agent
    clamor_cmd()
        .args(["new", "--folder", "test", "will be done"])
        .env("HOME", &home)
        .output()
        .unwrap();

    // Manually mark as done in state file
    let state_path = home.join(".clamor/state.json");
    let state_str = std::fs::read_to_string(&state_path).unwrap();
    let mut state: serde_json::Value = serde_json::from_str(&state_str).unwrap();

    if let Some(agents) = state["agents"].as_object_mut() {
        for (_, agent) in agents.iter_mut() {
            agent["state"] = serde_json::json!("done");
        }
    }
    std::fs::write(&state_path, serde_json::to_string_pretty(&state).unwrap()).unwrap();

    // Clean
    let output = clamor_cmd()
        .arg("clean")
        .env("HOME", &home)
        .output()
        .unwrap();
    assert!(output.status.success());
    assert!(String::from_utf8_lossy(&output.stdout).contains("1 finished agent(s)"));

    // Verify empty
    let output = clamor_cmd().arg("ls").env("HOME", &home).output().unwrap();
    assert!(String::from_utf8_lossy(&output.stdout).contains("No agents"));

    cleanup_test_env(&home);
}

// ── Multi-backend integration tests ──────────────────────────────

/// Create an isolated HOME directory with a YAML config containing multiple backends.
/// Returns the temp HOME path. The XDG config is at `{home}/.config/clamor/config.yaml`.
///
/// Because the host environment may have XDG_CONFIG_HOME set, all commands
/// spawned against this env must use `multi_backend_cmd()` or
/// `start_daemon_xdg()` which clear that variable so the config path
/// falls through to `$HOME/.config/clamor/`.
fn setup_multi_backend_env() -> PathBuf {
    let test_home =
        std::env::temp_dir().join(format!("clm-t-{}-{}", std::process::id(), rand_suffix()));
    std::fs::create_dir_all(test_home.join(".clamor")).unwrap();
    std::fs::create_dir_all(test_home.join(".config/clamor")).unwrap();

    let config_yaml = format!(
        r#"backends:
  claude-code:
    display_name: Claude
    spawn:
      cmd: [claude, "{{{{prompt}}}}"]
    resume:
      cmd: [claude, --resume, "{{{{resume_token}}}}"]
    capabilities:
      resume: true
  open-code:
    display_name: OpenCode
    spawn:
      cmd: [opencode, run, --prompt, "{{{{prompt}}}}"]
    capabilities:
      resume: false
folders:
  test:
    path: {path}
    backends: [claude-code, open-code]
"#,
        path = test_home.to_string_lossy()
    );

    std::fs::write(test_home.join(".config/clamor/config.yaml"), config_yaml).unwrap();

    test_home
}

/// Build a Command pre-configured for multi-backend tests.
/// Clears XDG_CONFIG_HOME so ClamorConfig falls back to $HOME/.config/clamor/.
fn multi_backend_cmd(home: &PathBuf) -> Command {
    let mut cmd = Command::new(clamor_bin());
    cmd.env("CLAMOR_DEBUG", "1")
        .env("HOME", home)
        .env_remove("XDG_CONFIG_HOME");
    cmd
}

/// Start the daemon with XDG_CONFIG_HOME cleared for multi-backend tests.
fn start_daemon_xdg(home: &PathBuf) {
    #[allow(clippy::zombie_processes)]
    let mut child = Command::new(clamor_bin())
        .arg("daemon")
        .env("HOME", home)
        .env("CLAMOR_DEBUG", "1")
        .env_remove("XDG_CONFIG_HOME")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn()
        .expect("failed to start daemon");

    let sock = home.join(".clamor/clamor.sock");
    for _ in 0..50 {
        if sock.exists() {
            return;
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    let _ = child.kill();
    panic!("daemon did not start within 5s");
}

#[test]
fn test_spawn_records_backend_id() {
    let home = setup_multi_backend_env();
    start_daemon_xdg(&home);

    let output = multi_backend_cmd(&home)
        .args(["new", "--folder", "test", "backend test"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "spawn failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    std::thread::sleep(Duration::from_millis(100));

    let state_path = home.join(".clamor/state.json");
    let state_str = std::fs::read_to_string(&state_path).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state_str).unwrap();

    let agents = state["agents"].as_object().unwrap();
    assert_eq!(agents.len(), 1, "expected 1 agent, got {}", agents.len());

    let agent = agents.values().next().unwrap();
    assert_eq!(
        agent["backend_id"].as_str().unwrap(),
        "claude-code",
        "default backend should be claude-code, got: {}",
        agent["backend_id"]
    );

    cleanup_test_env(&home);
}

#[test]
fn test_spawn_with_non_default_backend_selection() {
    let home = setup_multi_backend_env();

    // Pre-write state with open-code selected for the test folder
    let state = serde_json::json!({
        "agents": {},
        "folder_state": {
            "test": {
                "selected_backend": "open-code"
            }
        },
        "prompt_history": []
    });
    std::fs::write(
        home.join(".clamor/state.json"),
        serde_json::to_string_pretty(&state).unwrap(),
    )
    .unwrap();

    start_daemon_xdg(&home);

    let output = multi_backend_cmd(&home)
        .args(["new", "--folder", "test", "open-code test"])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "spawn failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    std::thread::sleep(Duration::from_millis(100));

    let state_path = home.join(".clamor/state.json");
    let state_str = std::fs::read_to_string(&state_path).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state_str).unwrap();

    let agents = state["agents"].as_object().unwrap();
    assert_eq!(agents.len(), 1, "expected 1 agent, got {}", agents.len());

    let agent = agents.values().next().unwrap();
    assert_eq!(
        agent["backend_id"].as_str().unwrap(),
        "open-code",
        "selected backend should be open-code, got: {}",
        agent["backend_id"]
    );

    cleanup_test_env(&home);
}

#[test]
fn test_adopt_uses_resumable_backend() {
    let home = setup_multi_backend_env();

    // Pre-set folder_state to select open-code (which has resume: false)
    let state = serde_json::json!({
        "agents": {},
        "folder_state": {
            "test": {
                "selected_backend": "open-code"
            }
        },
        "prompt_history": []
    });
    std::fs::write(
        home.join(".clamor/state.json"),
        serde_json::to_string_pretty(&state).unwrap(),
    )
    .unwrap();

    start_daemon_xdg(&home);

    let output = multi_backend_cmd(&home)
        .args([
            "adopt",
            "fake-session",
            "--folder",
            "test",
            "-d",
            "adopt test",
        ])
        .output()
        .unwrap();
    assert!(
        output.status.success(),
        "adopt failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );

    std::thread::sleep(Duration::from_millis(100));

    let state_path = home.join(".clamor/state.json");
    let state_str = std::fs::read_to_string(&state_path).unwrap();
    let state: serde_json::Value = serde_json::from_str(&state_str).unwrap();

    let agents = state["agents"].as_object().unwrap();
    assert_eq!(agents.len(), 1, "expected 1 agent, got {}", agents.len());

    let agent = agents.values().next().unwrap();
    assert_eq!(
        agent["backend_id"].as_str().unwrap(),
        "claude-code",
        "adopt should pick first resumable backend (claude-code), not the folder selection (open-code), got: {}",
        agent["backend_id"]
    );

    cleanup_test_env(&home);
}
