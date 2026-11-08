use anyhow::Result;
use serde_json::{Value, json};
use std::io::{Read, Write};
use std::net::TcpListener;
use std::path::Path;
use std::process::{Command, Stdio};

fn run(root: &Path, args: &[&str], input: &Value, expected: i32) -> Result<Vec<Value>> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_monika"))
        .args(args)
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(serde_json::to_string(input)?.as_bytes())?;
    let output = child.wait_with_output()?;
    assert_eq!(
        output.status.code(),
        Some(expected),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)?
        .lines()
        .map(|line| Ok(serde_json::from_str(line)?))
        .collect()
}

fn init(root: &Path) -> Result<()> {
    run(root, &["init"], &Value::Null, 0)?;
    Ok(())
}

#[test]
fn init_registers_bundled_extensions_and_enable_is_atomic() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    run(root, &["init", "--bare"], &Value::Null, 0)?;
    assert!(run(root, &["extension", "list"], &Value::Null, 0)?.is_empty());
    assert_eq!(
        run(root, &["extension", "list", "--bundled"], &Value::Null, 0)?.len(),
        monika::bundled::NAMES.len()
    );
    run(root, &["extension", "enable", "--all"], &Value::Null, 0)?;
    assert_eq!(
        run(root, &["extension", "list"], &Value::Null, 0)?.len(),
        monika::bundled::NAMES.len()
    );
    let path = root.join("monika.json");
    let before = std::fs::read(&path)?;
    run(
        root,
        &["extension", "enable", "text", "unknown"],
        &Value::Null,
        2,
    )?;
    assert_eq!(before, std::fs::read(&path)?);
    run(root, &["extension", "enable", "--all"], &Value::Null, 0)?;
    assert_eq!(before, std::fs::read(&path)?);
    let mut config: Value = serde_json::from_slice(&before)?;
    config["extensions"]["file"]["command"] = json!(["custom-provider"]);
    std::fs::write(&path, serde_json::to_vec(&config)?)?;
    let before = std::fs::read(&path)?;
    run(root, &["extension", "enable", "--all"], &Value::Null, 2)?;
    assert_eq!(before, std::fs::read(&path)?);
    Ok(())
}

#[test]
fn bundled_selectors_capture_refresh_and_compare_across_interpretations() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    init(root)?;
    let source = "fn run() { println!(\"雪\"); }\n";
    std::fs::write(root.join("sample.rs"), source)?;
    let target = json!({"extension":"code","resource":"sample.rs","query":{"symbol":"run"}});
    let whole = json!({"extension":"text","resource":"sample.rs"});
    let compare = run(root, &["compare"], &json!({"left":whole,"right":target}), 0)?;
    assert_eq!(compare[0]["relation"], "contains");
    let binary = run(
        root,
        &["read"],
        &json!({"extension":"file","resource":"sample.rs"}),
        0,
    )?;
    assert_eq!(
        binary[0]["region"]["content"]["sha256"],
        monika::model::digest(source.as_bytes())
    );
    let link = json!({"id":"quote","source":{"extension":"anchor","query":"report:implementation"},"target":target});
    std::fs::write(
        root.join("monika.links.json"),
        serde_json::to_vec(&json!({"version":1,"links":[link]}))?,
    )?;
    run(
        root,
        &["capture", "--link", "quote", "--endpoint", "both"],
        &Value::Null,
        0,
    )?;
    let links = run(root, &["links"], &Value::Null, 0)?;
    assert!(
        run(root, &["check"], &links[0], 0)?[0]["valid"]
            .as_bool()
            .unwrap()
    );
    std::fs::write(
        root.join("sample.rs"),
        format!("// moved\n{source}fn unrelated() {{}}\n"),
    )?;
    run(root, &["check"], &links[0], 0)?;
    std::fs::write(
        root.join("sample.rs"),
        "fn run() { println!(\"changed\"); }\n",
    )?;
    run(root, &["check"], &links[0], 1)?;
    let saved = run(
        root,
        &["read", "--at", "/target", "--captured"],
        &links[0],
        0,
    )?;
    assert!(
        saved[0]["region"]["content"]["text"]
            .as_str()
            .unwrap()
            .contains("雪")
    );
    let path = root.join("monika.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    config["resources"] = json!({"sample.rs":"refreshed"});
    std::fs::write(&path, serde_json::to_vec(&config)?)?;
    run(root, &["read"], &target, 2)?;
    run(root, &["refresh", "sample.rs"], &Value::Null, 0)?;
    let old = run(root, &["read"], &target, 0)?;
    std::fs::write(root.join("sample.rs"), "fn run() {}\n")?;
    assert_eq!(
        run(root, &["read"], &target, 0)?[0]["region"]["content"],
        old[0]["region"]["content"]
    );
    run(root, &["refresh", "sample.rs"], &Value::Null, 0)?;
    assert_ne!(
        run(root, &["read"], &target, 0)?[0]["region"]["content"],
        old[0]["region"]["content"]
    );
    Ok(())
}

#[test]
fn enabling_interpreters_preserves_custom_names_and_selects_a_bundled_comparator() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    run(root, &["init", "--bare"], &Value::Null, 0)?;
    let path = root.join("monika.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    let custom = json!({"command":["custom-text-extension"],"revision":"custom"});
    config["extensions"]["text"] = custom.clone();
    std::fs::write(&path, serde_json::to_vec(&config)?)?;
    run(
        root,
        &["extension", "enable", "file", "markdown", "code"],
        &Value::Null,
        0,
    )?;
    let enabled: Value = serde_json::from_slice(&std::fs::read(&path)?)?;
    assert_eq!(enabled["extensions"]["text"]["command"], custom["command"]);
    assert_eq!(enabled["comparators"]["monika.bytes.v1"], "markdown");
    std::fs::write(root.join("a.md"), "# A\nbody\n")?;
    let target = json!({"extension":"markdown","resource":"a.md"});
    assert_eq!(
        run(
            root,
            &["compare"],
            &json!({"left":target,"right":target}),
            0
        )?[0]["relation"],
        "equal"
    );
    Ok(())
}

#[test]
fn http_observations_distinguish_absence_and_errors() -> Result<()> {
    let directory = tempfile::tempdir()?;
    init(directory.path())?;
    for (status, body, expected) in [
        ("200 OK", "{\"answer\":42}", 0),
        ("404 Not Found", "missing", 2),
        ("503 Service Unavailable", "offline", 2),
    ] {
        let listener = TcpListener::bind("127.0.0.1:0")?;
        let address = listener.local_addr()?;
        let task = std::thread::spawn(move || -> Result<()> {
            let (mut stream, _) = listener.accept()?;
            stream.set_read_timeout(Some(std::time::Duration::from_secs(10)))?;
            let mut request = Vec::new();
            let mut buffer = [0; 1024];
            while !request.windows(4).any(|w| w == b"\r\n\r\n") {
                let count = stream.read(&mut buffer)?;
                anyhow::ensure!(count > 0, "closed HTTP request");
                request.extend_from_slice(&buffer[..count]);
            }
            write!(
                stream,
                "HTTP/1.1 {status}\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                body.len()
            )?;
            Ok(())
        });
        let selector = json!({"extension":"json","resource":format!("http://{address}/data"),"query":{"pointer":"/answer"}});
        let result = run(directory.path(), &["read"], &selector, expected)?;
        task.join().unwrap()?;
        if expected == 0 {
            assert_eq!(result[0]["region"]["content"]["values"], json!([42]));
        }
    }
    Ok(())
}

#[cfg(unix)]
#[test]
fn file_provider_rejects_fifo_before_opening_it() -> Result<()> {
    use std::os::unix::ffi::OsStrExt;
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    init(root)?;
    let path = std::ffi::CString::new(root.join("pipe").as_os_str().as_bytes())?;
    // The CString is live through the call and has a terminating NUL.
    assert_eq!(unsafe { libc::mkfifo(path.as_ptr(), 0o600) }, 0);
    let config_path = root.join("monika.json");
    let mut config: Value = serde_json::from_slice(&std::fs::read(&config_path)?)?;
    config["extensions"]["file"]["timeout_ms"] = json!(1000);
    std::fs::write(config_path, serde_json::to_vec(&config)?)?;
    let mut child = Command::new(env!("CARGO_BIN_EXE_monika"))
        .arg("read")
        .current_dir(root)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    child
        .stdin
        .take()
        .unwrap()
        .write_all(br#"{"extension":"text","resource":"pipe"}"#)?;
    let output = child.wait_with_output()?;
    assert_eq!(output.status.code(), Some(2));
    let error = String::from_utf8(output.stderr)?;
    assert!(error.contains("not a regular file"), "{error}");
    assert!(!error.contains("timed out"), "{error}");
    Ok(())
}

#[test]
fn git_reads_committed_blob_without_changing_worktree() -> Result<()> {
    let directory = tempfile::tempdir()?;
    let root = directory.path();
    init(root)?;
    std::fs::write(root.join("a +雪.txt"), "committed\n")?;
    std::fs::create_dir(root.join("dir"))?;
    std::fs::write(
        root.join("dir/only.txt"),
        "a directory is not its first file",
    )?;
    let git = |args: &[&str]| -> Result<Vec<u8>> {
        let out = Command::new("git").args(args).current_dir(root).output()?;
        anyhow::ensure!(
            out.status.success(),
            "{}",
            String::from_utf8_lossy(&out.stderr)
        );
        Ok(out.stdout)
    };
    git(&["init", "-q"])?;
    git(&["add", "--", "a +雪.txt", "dir/only.txt"])?;
    git(&[
        "-c",
        "user.name=Monika Test",
        "-c",
        "user.email=test@example.invalid",
        "commit",
        "-qm",
        "fixture",
    ])?;
    std::fs::write(root.join("a +雪.txt"), "working copy\n")?;
    let before = git(&["status", "--porcelain"])?;
    let selector = json!({"extension":"text","resource":"git:a%20+%E9%9B%AA.txt?rev=HEAD"});
    let result = run(root, &["read"], &selector, 0)?;
    assert_eq!(result[0]["region"]["content"]["text"], "committed\n");
    assert_eq!(before, git(&["status", "--porcelain"])?);
    for resource in [
        "git:missing.txt?rev=HEAD",
        "git:a%20+%E9%9B%AA.txt?rev=missing-revision",
        "git:%2E%2E/secret?rev=HEAD",
        "git:*.txt?rev=HEAD",
        "git:dir?rev=HEAD",
        "git:dir/?rev=HEAD",
        "git:dir%2F?rev=HEAD",
    ] {
        run(
            root,
            &["read"],
            &json!({"extension":"text","resource":resource}),
            2,
        )?;
    }
    // A Monika project may live below the repository root.
    run(&root.join("dir"), &["--root", ".", "init"], &Value::Null, 0)?;
    assert_eq!(
        run(
            &root.join("dir"),
            &["read"],
            &json!({"extension":"text","resource":"git:only.txt?rev=HEAD"}),
            0
        )?[0]["region"]["content"]["text"],
        "a directory is not its first file"
    );
    Ok(())
}

#[test]
fn executable_bundled_documentation_works() -> Result<()> {
    let output = Command::new("python3")
        .arg(format!(
            "{}/examples/bundled_demo.py",
            env!("CARGO_MANIFEST_DIR")
        ))
        .args(["--monika", env!("CARGO_BIN_EXE_monika")])
        .output()?;
    assert!(
        output.status.success(),
        "{}\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    Ok(())
}
