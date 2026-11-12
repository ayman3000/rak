//! Tests of the `rak` command as a whole program: what only shows when the
//! process itself starts and ends.

#[cfg(unix)]
mod unix {
    use std::path::{Path, PathBuf};
    use std::process::{Command, Stdio};
    use std::time::{Duration, Instant};

    fn temp_dir(label: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("rak-cli-{label}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        dir
    }

    fn alive(pid: &str) -> bool {
        Command::new("kill").args(["-0", pid]).stderr(Stdio::null()).status().is_ok_and(|s| s.success())
    }

    /// A stand-in MCP server that writes its pid to `pid_file`, answers the
    /// handshake and lists one tool, then waits for more. Like some real ones,
    /// it keeps running after its input closes.
    fn fake_server(pid_file: &Path) -> String {
        format!(
            r#"echo $$ > '{}'
n=0
while IFS= read -r line; do
  case "$line" in *'"id"'*) n=$((n+1)) ;; *) continue ;; esac
  case "$line" in
    *'"initialize"'*) echo '{{"jsonrpc":"2.0","id":'$n',"result":{{"protocolVersion":"2025-06-18","capabilities":{{"tools":{{}}}},"serverInfo":{{"name":"fake","version":"1"}}}}}}' ;;
    *'"tools/list"'*) echo '{{"jsonrpc":"2.0","id":'$n',"result":{{"tools":[{{"name":"look","description":"Looks.","inputSchema":{{"type":"object","properties":{{}}}}}}]}}}}' ;;
  esac
done
sleep 30"#,
            pid_file.display()
        )
    }

    /// A project with one MCP server configured, and a home of its own so the
    /// user's real config is never read. Returns (dir, project, home, pid file).
    fn project_with_server(label: &str) -> (PathBuf, PathBuf, PathBuf, PathBuf) {
        let dir = temp_dir(label);
        let (project, home, pid_file) = (dir.join("project"), dir.join("home"), dir.join("server.pid"));
        std::fs::create_dir_all(project.join(".rak")).unwrap();
        std::fs::create_dir_all(&home).unwrap();
        let config = serde_json::json!({ "mcpServers": { "fake": { "command": "sh", "args": ["-c", fake_server(&pid_file)] } } });
        std::fs::write(project.join(".rak/mcp.json"), config.to_string()).unwrap();
        (dir, project, home, pid_file)
    }

    fn rak(home: &Path) -> Command {
        let mut command = Command::new(env!("CARGO_BIN_EXE_rak"));
        command.env("HOME", home).env("XDG_CONFIG_HOME", home.join(".config")).stdin(Stdio::piped());
        command
    }

    /// The server rak started has stopped (or stops within a few seconds).
    fn assert_server_stopped(pid_file: &Path) {
        let pid = std::fs::read_to_string(pid_file).expect("the server was started").trim().to_string();
        let deadline = Instant::now() + Duration::from_secs(3);
        while alive(&pid) && Instant::now() < deadline {
            std::thread::sleep(Duration::from_millis(50));
        }
        let still_alive = alive(&pid);
        if still_alive {
            let _ = Command::new("kill").args(["-9", &pid]).status();
        }
        assert!(!still_alive, "the MCP server (pid {pid}) outlived rak");
    }

    #[test]
    fn a_usage_error_after_the_mcp_servers_start_stops_them() {
        let (dir, project, home, pid_file) = project_with_server("mcp-exit");
        // No request and nothing piped in: "nothing to do", after the servers started.
        let output = rak(&home)
            .args(["-m", "test-model", "--base-url", "http://127.0.0.1:9/v1", "--no-skills", "-C"])
            .arg(&project)
            .output()
            .unwrap();
        assert_eq!(output.status.code(), Some(2), "{}", String::from_utf8_lossy(&output.stderr));
        assert!(String::from_utf8_lossy(&output.stderr).contains("nothing to do"));
        assert_server_stopped(&pid_file);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn quitting_with_a_second_ctrl_c_stops_the_mcp_servers() {
        let (dir, project, home, pid_file) = project_with_server("mcp-ctrl-c");
        // A model server that takes the chat request and never answers, so rak
        // waits mid-request. Anything else (Ollama's /api/show) gets a 404.
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let url = format!("http://{}/v1", listener.local_addr().unwrap());
        let (chat_sender, chat_arrived) = std::sync::mpsc::channel();
        std::thread::spawn(move || {
            use std::io::{BufRead, BufReader, Write};
            for mut stream in listener.incoming().map_while(Result::ok) {
                let mut first = String::new();
                let _ = BufReader::new(&stream).read_line(&mut first);
                if first.contains("/chat/completions") {
                    let _ = chat_sender.send(stream); // held open, never answered
                } else {
                    let _ = stream.write_all(b"HTTP/1.1 404 Not Found\r\nContent-Length: 0\r\nConnection: close\r\n\r\n");
                }
            }
        });
        let mut child = rak(&home)
            .args(["-m", "test-model", "--base-url", &url, "--context", "100000", "--no-skills", "-q", "-C"])
            .arg(&project)
            .arg("say hi")
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .spawn()
            .unwrap();
        let _held = chat_arrived.recv_timeout(Duration::from_secs(10)).expect("rak sent its request");
        assert!(pid_file.exists(), "the server started before the request");

        let pid = child.id().to_string();
        let interrupt = || Command::new("kill").args(["-INT", &pid]).status().unwrap();
        interrupt(); // asks the request to stop; the silent model keeps it waiting
        std::thread::sleep(Duration::from_millis(300));
        interrupt(); // quits rak
        let deadline = Instant::now() + Duration::from_secs(5);
        let status = loop {
            if let Some(status) = child.try_wait().unwrap() {
                break status;
            }
            assert!(Instant::now() < deadline, "rak did not quit");
            std::thread::sleep(Duration::from_millis(20));
        };
        assert_eq!(status.code(), Some(130));
        assert_server_stopped(&pid_file);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn rak_mcp_list_stops_the_servers_it_tried() {
        let (dir, project, home, pid_file) = project_with_server("mcp-list");
        let output = rak(&home).args(["mcp", "list"]).current_dir(&project).output().unwrap();
        assert!(String::from_utf8_lossy(&output.stdout).contains("✓ 1 tool"), "{}", String::from_utf8_lossy(&output.stdout));
        assert_server_stopped(&pid_file);
        let _ = std::fs::remove_dir_all(&dir);
    }
}
