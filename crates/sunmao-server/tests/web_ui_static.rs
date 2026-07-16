//! Structural + HTTP checks for embedded Web UI (no LLM).

use std::net::SocketAddr;
use std::path::PathBuf;
use std::process::{Child, Command, Stdio};
use std::time::Duration;

fn ui_files() -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static")
}

#[test]
fn web_ui_static_assets_exist() {
    let dir = ui_files();
    assert!(dir.join("index.html").is_file(), "missing index.html");
    assert!(dir.join("app.js").is_file(), "missing app.js");
    assert!(dir.join("styles.css").is_file(), "missing styles.css");
    let html = std::fs::read_to_string(dir.join("index.html")).unwrap();
    assert!(html.contains("sunmao"), "index should brand sunmao");
    assert!(
        html.contains("不内嵌 LLM") || html.contains("非 Agent"),
        "UI should state non-Agent / no LLM boundary"
    );
    let js = std::fs::read_to_string(dir.join("app.js")).unwrap();
    assert!(js.contains("/v1/projects"), "app must call real API");
    assert!(js.contains("X-Sunmao-Actor"), "app must send actor header");
}

fn free_port() -> u16 {
    let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
    listener.local_addr().unwrap().port()
}

fn wait_health(base: &str) -> bool {
    for _ in 0..40 {
        if let Ok(resp) = ureq_get(&format!("{base}/health")) {
            if resp == "ok" {
                return true;
            }
        }
        std::thread::sleep(Duration::from_millis(150));
    }
    false
}

/// Minimal GET without extra deps: use std + subprocess curl if needed.
fn ureq_get(url: &str) -> Result<String, String> {
    let out = Command::new("curl")
        .args(["-sf", url])
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "curl failed: {}",
            String::from_utf8_lossy(&out.stderr)
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout).to_string())
}

struct ChildGuard(Child);
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let _ = self.0.kill();
        let _ = self.0.wait();
    }
}

#[test]
fn web_ui_served_when_server_runs() {
    let db = match std::env::var("DATABASE_URL") {
        Ok(u) if !u.is_empty() => u,
        _ => {
            eprintln!("skip web_ui_served_when_server_runs: DATABASE_URL not set");
            return;
        }
    };
    let port = free_port();
    let addr: SocketAddr = format!("127.0.0.1:{port}").parse().unwrap();
    let bin = PathBuf::from(env!("CARGO_MANIFEST_DIR"))
        .join("../../target/debug/sunmao-server");
    let bin = if bin.exists() {
        bin
    } else {
        // try workspace target
        PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("../../../target/debug/sunmao-server")
    };
    assert!(
        bin.exists(),
        "sunmao-server binary missing at {}; run cargo build -p sunmao-server first",
        bin.display()
    );

    let child = Command::new(&bin)
        .args(["--db", &db, "--listen", &addr.to_string()])
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .spawn()
        .expect("spawn sunmao-server");
    let _guard = ChildGuard(child);

    let base = format!("http://{addr}");
    assert!(wait_health(&base), "server health timeout");

    let index = ureq_get(&format!("{base}/ui/")).expect("GET /ui/");
    assert!(
        index.contains("sunmao") || index.contains("榫"),
        "UI HTML missing brand: {}",
        &index[..index.len().min(200)]
    );
    let js = ureq_get(&format!("{base}/ui/app.js")).expect("GET /ui/app.js");
    assert!(js.contains("loadProjects") || js.contains("/v1/projects"));
    let css = ureq_get(&format!("{base}/ui/styles.css")).expect("GET /ui/styles.css");
    assert!(css.contains("--bg") || css.contains("body"));

    // root redirects to /ui/
    let status = Command::new("curl")
        .args(["-s", "-o", "/dev/null", "-w", "%{http_code}", &format!("{base}/")])
        .output()
        .unwrap();
    let code = String::from_utf8_lossy(&status.stdout);
    assert!(
        code == "307" || code == "302" || code == "301" || code == "200",
        "unexpected redirect code {code}"
    );
}
