use std::{
    fs,
    io::{BufRead, BufReader, Write},
    path::Path,
    process::{Child, ChildStdin, Command, Output, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

use serde_json::{Value, json};

fn command(directory: &Path) -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_pane-md"));
    command.current_dir(directory);
    command
}

fn run(directory: &Path, args: &[&str]) -> Output {
    command(directory).args(args).output().unwrap()
}

fn fixture(directory: &Path) {
    let source = "\u{feff}# Тест\r\n\r\nRead **quote**.\r\n\r\n<!-- pane-md:review {\"v\":1,\"id\":\"review-1\",\"quote\":\"quote\",\"anchor\":{\"heading\":\"Тест\",\"block\":\"paragraph\",\"prefix\":\"Read \",\"suffix\":\".\"},\"text\":\"Добрый день\",\"status\":\"open\"} -->\r\n\r\n[wrong](#missing) and [valid](#%D1%82%D0%B5%D1%81%D1%82).\r\n";
    fs::write(directory.join("-заметка.md"), source).unwrap();
}

#[test]
fn cli_reports_an_issue_without_mutating_bom_or_crlf() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    fixture(cwd);
    let path = cwd.join("-заметка.md");
    let before = fs::read(&path).unwrap();
    fs::write(cwd.join("clean.md"), "# Good\n\n[Here](#good)\n").unwrap();

    let output = run(cwd, &["--lint", "--json", "--", "clean.md", "-заметка.md"]);
    assert_eq!(output.status.code(), Some(1));
    assert!(output.stderr.is_empty());
    let reports: Vec<Value> = serde_json::from_slice(&output.stdout).unwrap();
    assert!(reports[0]["issues"].as_array().unwrap().is_empty());
    let [issue] = reports[1]["issues"].as_array().unwrap().as_slice() else {
        panic!("expected one broken link: {}", reports[1]);
    };
    assert_eq!(issue["code"], "anchor-missing");
    assert_eq!(issue["line"], 7);
    assert_eq!(issue["column"], 2);
    assert!(run(cwd, &["--lint", "clean.md"]).status.success());

    let comments = run(cwd, &["--list-comments", "--", "-заметка.md"]);
    assert!(comments.status.success());
    let comments: Value = serde_json::from_slice(&comments.stdout).unwrap();
    assert_eq!(comments["comments"][0]["quote"], "quote");
    assert_eq!(comments["comments"][0]["state"], "attached");
    assert_eq!(comments["comments"][0]["text"], "Добрый день");
    assert!(comments["diagnostics"].as_array().unwrap().is_empty());
    assert_eq!(fs::read(path).unwrap(), before);
}

#[test]
fn cli_distinguishes_usage_and_file_errors_from_lint_findings() {
    let directory = tempfile::tempdir().unwrap();
    let cwd = directory.path();
    fs::write(cwd.join("nul.md"), b"# Text\0").unwrap();
    fs::write(cwd.join("invalid.md"), b"\xff").unwrap();
    for input in ["missing.md", ".", "nul.md", "invalid.md", "--unknown"] {
        let output = run(cwd, &["--lint", "--json", input]);
        assert_eq!(output.status.code(), Some(2), "{input}");
        assert!(output.stdout.is_empty(), "{input}");
        assert!(!output.stderr.is_empty(), "{input}");
    }
    assert!(run(cwd, &["--help"]).status.success());
}

struct McpClient {
    child: Child,
    input: Option<ChildStdin>,
    lines: mpsc::Receiver<String>,
}

impl McpClient {
    fn start(mut command: Command) -> Self {
        let mut child = command
            .arg("--mcp")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let input = child.stdin.take().unwrap();
        let reader = BufReader::new(child.stdout.take().unwrap());
        let (send, lines) = mpsc::channel();
        thread::spawn(move || {
            for line in reader.lines() {
                let Ok(line) = line else { break };
                if send.send(line).is_err() {
                    break;
                }
            }
        });
        Self {
            child,
            input: Some(input),
            lines,
        }
    }

    fn send(&mut self, message: Value) {
        let input = self.input.as_mut().unwrap();
        writeln!(input, "{message}").unwrap();
        input.flush().unwrap();
    }

    fn request(&mut self, id: u64, method: &str, params: Value) -> Value {
        self.send(json!({ "jsonrpc": "2.0", "id": id, "method": method, "params": params }));
        let line = self.lines.recv_timeout(Duration::from_secs(10)).unwrap();
        let response: Value = serde_json::from_str(&line).unwrap();
        assert_eq!(response["id"], id, "{response}");
        response
    }

    fn finish(mut self) {
        drop(self.input.take());
        let deadline = Instant::now() + Duration::from_secs(10);
        loop {
            if let Some(status) = self.child.try_wait().unwrap() {
                assert!(status.success());
                return;
            }
            assert!(Instant::now() < deadline, "MCP did not exit after EOF");
            thread::sleep(Duration::from_millis(10));
        }
    }
}

impl Drop for McpClient {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}

#[test]
fn mcp_serves_lint_and_comments_over_stdio() {
    let directory = tempfile::tempdir().unwrap();
    fixture(directory.path());
    let desktop = cfg!(any(
        windows,
        all(
            feature = "desktop-gui",
            any(target_os = "linux", target_os = "macos")
        )
    ));
    let mut server = command(directory.path());
    if cfg!(target_os = "linux") {
        server.env_remove("DISPLAY").env_remove("WAYLAND_DISPLAY");
    }
    let mut mcp = McpClient::start(server);
    let init = mcp.request(
        1,
        "initialize",
        json!({
            "protocolVersion":"2025-11-25",
            "capabilities":{},
            "clientInfo":{"name":"rust-test","version":"1"}
        }),
    );
    assert!(
        init["result"]["capabilities"]["tools"].is_object(),
        "{init}"
    );
    mcp.send(json!({"jsonrpc":"2.0","method":"notifications/initialized"}));

    let list = mcp.request(2, "tools/list", json!({}));
    let tools = list["result"]["tools"].as_array().unwrap();
    let names = tools
        .iter()
        .map(|tool| tool["name"].as_str().unwrap())
        .collect::<Vec<_>>();
    assert!(names.contains(&"lint_document") && names.contains(&"list_comments"));
    assert_eq!(names.contains(&"open_document"), desktop);
    assert!(
        mcp.request(
            3,
            "tools/call",
            json!({
                "name":"lint_document", "arguments":{"path":"missing.md"}
            })
        )["error"]
            .is_object()
    );

    let lint = mcp.request(
        4,
        "tools/call",
        json!({
            "name":"lint_document", "arguments":{"path":"-заметка.md"}
        }),
    );
    assert_eq!(
        lint["result"]["structuredContent"]["issues"][0]["code"],
        "anchor-missing"
    );
    let comments = mcp.request(
        5,
        "tools/call",
        json!({
            "name":"list_comments", "arguments":{"path":"-заметка.md"}
        }),
    );
    assert_eq!(
        comments["result"]["structuredContent"]["comments"][0]["state"],
        "attached"
    );
    fs::write(directory.path().join("target.md"), "# Visible\n").unwrap();
    fs::write(
        directory.path().join("local.md"),
        "[Good](target.md#visible) [Wrong](target.md#hidden) [Missing](photo.png)\n",
    )
    .unwrap();
    let local = mcp.request(
        8,
        "tools/call",
        json!({"name": "lint_document", "arguments": {"path": "local.md"}}),
    );
    let issues = &local["result"]["structuredContent"]["issues"];
    assert_eq!(issues[0]["code"], "anchor-missing");
    assert_eq!(issues[1]["code"], "local-target-missing");
    assert_eq!(issues.as_array().unwrap().len(), 2);
    let cli = run(directory.path(), &["--lint", "--json", "local.md"]);
    assert_eq!(cli.status.code(), Some(1));
    let reports: Value = serde_json::from_slice(&cli.stdout).unwrap();
    assert_eq!(&reports[0]["issues"], issues);
    if desktop {
        let open = |mcp: &mut McpClient, id, path| {
            mcp.request(
                id,
                "tools/call",
                json!({"name": "open_document", "arguments": {"path": path}}),
            )
        };
        assert!(open(&mut mcp, 6, "missing.md")["error"].is_object());
        if cfg!(target_os = "linux") {
            let error = open(&mut mcp, 7, "-заметка.md");
            assert!(
                error["error"]["message"].as_str().unwrap().contains("X11"),
                "{error}"
            );
        }
    }
    mcp.finish();
}

#[cfg(windows)]
#[test]
fn windows_executable_contains_icon_at_taskbar_and_explorer_sizes() {
    use windows::{
        Win32::{
            Foundation::FreeLibrary,
            System::LibraryLoader::{
                FindResourceW, LOAD_LIBRARY_AS_DATAFILE, LoadLibraryExW, LoadResource,
                LockResource, SizeofResource,
            },
            UI::WindowsAndMessaging::RT_GROUP_ICON,
        },
        core::PCWSTR,
    };

    let exe = env!("CARGO_BIN_EXE_pane-md")
        .encode_utf16()
        .chain(Some(0))
        .collect::<Vec<_>>();
    unsafe {
        let module = LoadLibraryExW(PCWSTR(exe.as_ptr()), None, LOAD_LIBRARY_AS_DATAFILE).unwrap();
        let resource = FindResourceW(
            Some(module),
            PCWSTR(std::ptr::without_provenance(1)),
            RT_GROUP_ICON,
        );
        assert!(
            !resource.is_invalid(),
            "icon group is missing from the executable"
        );
        let length = SizeofResource(Some(module), resource) as usize;
        let handle = LoadResource(Some(module), resource).unwrap();
        let data = LockResource(handle).cast::<u8>();
        assert!(!data.is_null() && length >= 6);
        let data = std::slice::from_raw_parts(data, length);
        let widths = data[6..]
            .as_chunks::<14>()
            .0
            .iter()
            .map(|entry| entry[0])
            .collect::<Vec<_>>();
        assert!(widths.contains(&16) && widths.contains(&32) && widths.contains(&0));
        FreeLibrary(module).unwrap();
    }
}
