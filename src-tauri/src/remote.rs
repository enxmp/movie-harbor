//! Opt-in, authenticated loopback control through a user-configured SSH tunnel.
use super::*;
use std::io::{Read, Write};
use std::net::{TcpListener, TcpStream};
use std::process::Child;

const PORT: u16 = 38473;
const MAX_BODY: usize = 1024 * 1024;

struct ServerSession {
    stop: Arc<AtomicBool>,
}
impl Drop for ServerSession {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
    }
}
#[derive(Default)]
pub struct RemoteServerState(Mutex<Option<ServerSession>>);
struct Client {
    child: Child,
    port: u16,
    token: String,
    local_root: PathBuf,
    remote_root: String,
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.child.kill();
        let _ = self.child.wait();
    }
}
#[derive(Default)]
pub struct RemoteClientState(Mutex<Option<Client>>);
#[derive(Deserialize)]
pub struct Connection {
    host: String,
    user: String,
    key_path: String,
    local_root: String,
    token: String,
}

fn scoped_path(value: &str, root: &Path) -> Result<PathBuf, String> {
    let path = fs::canonicalize(value).map_err(|e| format!("Shared path unavailable: {e}"))?;
    if !path.starts_with(root) {
        return Err("Path is outside the folder shared by the Mac".into());
    }
    Ok(path)
}
fn queue_is_shared(q: &Queue, root: &Path) -> bool {
    q.entries.lock().unwrap().iter().all(|e| {
        let p = Path::new(&e.job.source);
        fs::canonicalize(p)
            .or_else(|_| fs::canonicalize(p.parent().unwrap_or(p)))
            .map(|p| p.starts_with(root))
            .unwrap_or(false)
    })
}
fn handle(action: &str, payload: Value, q: Arc<Queue>, root: &Path) -> Result<Value, String> {
    match action {
        "environment" => {
            let mut env = tauri::async_runtime::block_on(environment())?;
            env["sharedRoot"] = json!(root.to_string_lossy());
            Ok(env)
        }
        "inspect" => {
            let p = scoped_path(payload["path"].as_str().ok_or("Missing path")?, root)?;
            probe(&p.to_string_lossy(), &executable("ffprobe"))
        }
        "status" | "queue" | "cancel" => {
            if !queue_is_shared(&q, root) {
                return Err("The local queue contains files outside the shared folder".into());
            }
            if action == "cancel" {
                stop_queue(&q);
                return Ok(Value::Null);
            }
            let entries = q.entries.lock().unwrap();
            if action == "queue" {
                Ok(json!(entries.iter().map(|e|json!({"id":e.id,"job":e.job,"progress":e.state.progress.lock().unwrap().clone()})).collect::<Vec<_>>()))
            } else {
                Ok(
                    json!({"running":q.active.load(Ordering::SeqCst),"jobs":entries.iter().map(|e|(e.id.clone(),e.state.progress.lock().unwrap().clone())).collect::<std::collections::HashMap<_,_>>()}),
                )
            }
        }
        "start" => {
            let mut jobs: Vec<Submission> =
                serde_json::from_value(payload["jobs"].clone()).map_err(|e| e.to_string())?;
            if jobs.len() > 1000 {
                return Err("Too many jobs".into());
            }
            let parallel = payload["parallel"].as_u64().ok_or("Missing concurrency")? as usize;
            for item in &mut jobs {
                item.job.source = scoped_path(&item.job.source, root)?
                    .to_string_lossy()
                    .into_owned();
                if !item.job.folder.is_empty() {
                    item.job.folder = scoped_path(&item.job.folder, root)?
                        .to_string_lossy()
                        .into_owned();
                }
                item.job.ffmpeg = executable("ffmpeg");
                item.job.ffprobe = executable("ffprobe");
                item.job.video = if item.job.video == "copy" {
                    "copy"
                } else {
                    "apple"
                }
                .into();
                item.job.split_encode = false;
            }
            launch_queue(jobs, parallel, q)?;
            Ok(Value::Null)
        }
        _ => Err("Unknown remote action".into()),
    }
}
fn token_matches(expected: &str, received: &str) -> bool {
    if expected.len() != received.len() {
        return false;
    }
    expected
        .bytes()
        .zip(received.bytes())
        .fold(0u8, |diff, (a, b)| diff | (a ^ b))
        == 0
}
fn read_request(stream: &mut TcpStream, token: &str) -> Result<Value, String> {
    let deadline = std::time::Instant::now() + Duration::from_secs(3);
    fn read_until(
        stream: &mut TcpStream,
        buffer: &mut [u8],
        deadline: std::time::Instant,
    ) -> Result<(), String> {
        let mut offset = 0;
        while offset < buffer.len() {
            let remaining = deadline
                .checked_duration_since(std::time::Instant::now())
                .filter(|d| !d.is_zero())
                .ok_or("Request timed out")?;
            stream
                .set_read_timeout(Some(remaining))
                .map_err(|e| e.to_string())?;
            let n = stream
                .read(&mut buffer[offset..])
                .map_err(|e| e.to_string())?;
            if n == 0 {
                return Err("Incomplete request".into());
            }
            offset += n;
        }
        Ok(())
    }
    let mut header = Vec::new();
    let mut byte = [0u8; 1];
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 8192 {
            return Err("Headers too large".into());
        }
        read_until(stream, &mut byte, deadline)?;
        header.push(byte[0]);
    }
    let header = std::str::from_utf8(&header).map_err(|_| "Invalid headers")?;
    let mut lines = header.split("\r\n");
    if lines.next() != Some("POST /rpc HTTP/1.1") {
        return Err("Invalid request".into());
    }
    let mut fields = std::collections::HashMap::new();
    for line in lines.filter(|line| !line.is_empty()) {
        let (name, value) = line.split_once(':').ok_or("Invalid header")?;
        if fields
            .insert(name.to_ascii_lowercase(), value.trim())
            .is_some()
        {
            return Err("Duplicate header".into());
        }
    }
    if fields.contains_key("origin") || fields.contains_key("transfer-encoding") {
        return Err("Browser and chunked requests are not supported".into());
    }
    if fields.get("content-type") != Some(&"application/json") {
        return Err("JSON required".into());
    }
    let auth = fields
        .get("authorization")
        .and_then(|v| v.strip_prefix("Bearer "))
        .unwrap_or("");
    if !token_matches(token, auth) {
        return Err("Unauthorized".into());
    }
    let size = fields
        .get("content-length")
        .and_then(|n| n.parse::<usize>().ok())
        .filter(|n| *n <= MAX_BODY)
        .ok_or("Invalid request length")?;
    let mut body = vec![0; size];
    read_until(stream, &mut body, deadline)?;
    serde_json::from_slice(&body).map_err(|_| "Invalid JSON".into())
}
#[tauri::command]
pub fn enable_remote(
    root: String,
    state: tauri::State<RemoteServerState>,
    queue: tauri::State<Arc<Queue>>,
) -> Result<Value, String> {
    if !cfg!(target_os = "macos") {
        return Err("Hosting is supported by the macOS app".into());
    }
    let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
    if !root.is_dir() {
        return Err("Choose a shared folder".into());
    }
    let mut slot = state.0.lock().unwrap();
    if slot.is_some() {
        return Err("Remote control is already enabled".into());
    }
    let (session, result) = start_server(root, queue.inner().clone(), PORT)?;
    *slot = Some(session);
    Ok(result)
}
fn start_server(root: PathBuf, q: Arc<Queue>, port: u16) -> Result<(ServerSession, Value), String> {
    let listener = TcpListener::bind(("127.0.0.1", port))
        .map_err(|e| format!("Cannot enable remote control: {e}"))?;
    let port = listener.local_addr().map_err(|e| e.to_string())?.port();
    listener.set_nonblocking(true).map_err(|e| e.to_string())?;
    let mut random = [0u8; 32];
    getrandom::fill(&mut random).map_err(|e| e.to_string())?;
    let token: String = random.iter().map(|b| format!("{b:02x}")).collect();
    let result = json!({"token":token,"root":root.to_string_lossy(),"port":port});
    let stop = Arc::new(AtomicBool::new(false));
    let flag = stop.clone();
    thread::spawn(move || {
        while !flag.load(Ordering::SeqCst) {
            match listener.accept() {
                Ok((mut stream, _)) => {
                    let _ = stream.set_read_timeout(Some(Duration::from_secs(3)));
                    let _ = stream.set_write_timeout(Some(Duration::from_secs(3)));
                    let response = read_request(&mut stream, &token).and_then(|v| {
                        handle(
                            v["action"].as_str().unwrap_or(""),
                            v["payload"].clone(),
                            q.clone(),
                            &root,
                        )
                    });
                    let body = match response {
                        Ok(value) => json!({"ok":true,"value":value}),
                        Err(error) => json!({"ok":false,"error":error}),
                    }
                    .to_string();
                    let _=write!(stream,"HTTP/1.1 200 OK\r\nConnection: close\r\nContent-Type: application/json\r\nContent-Length: {}\r\n\r\n{}",body.len(),body);
                }
                Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => {
                    thread::sleep(Duration::from_millis(50))
                }
                Err(_) => break,
            }
        }
    });
    Ok((ServerSession { stop }, result))
}
#[tauri::command]
pub fn disable_remote(state: tauri::State<RemoteServerState>) {
    state.0.lock().unwrap().take();
}

fn rpc(port: u16, token: &str, action: &str, payload: Value) -> Result<Value, String> {
    let mut stream = TcpStream::connect_timeout(
        &format!("127.0.0.1:{port}").parse().unwrap(),
        Duration::from_secs(2),
    )
    .map_err(|e| e.to_string())?;
    stream
        .set_read_timeout(Some(Duration::from_secs(30)))
        .map_err(|e| e.to_string())?;
    stream
        .set_write_timeout(Some(Duration::from_secs(5)))
        .map_err(|e| e.to_string())?;
    let body = json!({"action":action,"payload":payload}).to_string();
    if body.len() > MAX_BODY {
        return Err("Request too large".into());
    }
    write!(stream,"POST /rpc HTTP/1.1\r\nHost: 127.0.0.1\r\nConnection: close\r\nContent-Type: application/json\r\nAuthorization: Bearer {token}\r\nContent-Length: {}\r\n\r\n{}",body.len(),body).map_err(|e|e.to_string())?;
    let mut raw = String::new();
    (&mut stream)
        .take((MAX_BODY + 8193) as u64)
        .read_to_string(&mut raw)
        .map_err(|e| e.to_string())?;
    if raw.len() > MAX_BODY + 8192 {
        return Err("Response too large".into());
    }
    let body = raw.split_once("\r\n\r\n").ok_or("Invalid response")?.1;
    let result: Value = serde_json::from_str(body).map_err(|_| "Invalid remote response")?;
    if result["ok"] == true {
        Ok(result["value"].clone())
    } else {
        Err(result["error"]
            .as_str()
            .unwrap_or("Remote request failed")
            .into())
    }
}
fn valid_connection(c: &Connection) -> bool {
    !c.host.is_empty()
        && !c.host.starts_with('-')
        && c.host
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'.' || b == b'-')
        && !c.user.is_empty()
        && !c.user.starts_with('-')
        && c.user
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-' || b == b'.')
        && c.token.len() == 64
        && c.token.bytes().all(|b| b.is_ascii_hexdigit())
}
#[tauri::command]
pub async fn remote_connect(
    config: Connection,
    state: tauri::State<'_, RemoteClientState>,
) -> Result<Value, String> {
    if !valid_connection(&config) {
        return Err("Enter a valid hostname, SSH username, and 64-character access token".into());
    }
    let key = fs::canonicalize(&config.key_path).map_err(|_| "SSH key file not found")?;
    let local_root =
        fs::canonicalize(&config.local_root).map_err(|_| "Local shared folder not found")?;
    if !key.is_file() || !local_root.is_dir() {
        return Err("Choose an SSH key file and a shared folder".into());
    }
    let (client,env)=tauri::async_runtime::spawn_blocking(move||->Result<(Client,Value),String>{
        let listener=TcpListener::bind(("127.0.0.1",0)).map_err(|e|e.to_string())?;
        let port=listener.local_addr().map_err(|e|e.to_string())?.port();drop(listener);
        let destination=format!("{}@{}",config.user,config.host);
        let forward=format!("127.0.0.1:{port}:127.0.0.1:{PORT}");
        let child=command("ssh").args(["-i",&key.to_string_lossy(),"-o","BatchMode=yes","-o","StrictHostKeyChecking=yes","-o","ExitOnForwardFailure=yes","-o","ConnectTimeout=8","-L",&forward,"-N",&destination]).stdin(Stdio::null()).stdout(Stdio::null()).stderr(Stdio::null()).spawn().map_err(|e|e.to_string())?;
        let mut client=Client{child,port,token:config.token,local_root,remote_root:String::new()};
        for _ in 0..50{
            if client.child.try_wait().map_err(|e|e.to_string())?.is_some(){return Err("SSH connection failed. Check the key, user, and trusted host entry using SSH first.".into())}
            if let Ok(env)=rpc(port,&client.token,"environment",json!({})){
                client.remote_root=env["sharedRoot"].as_str().filter(|p|p.starts_with('/')).ok_or("Mac shared folder unavailable")?.into();
                return Ok((client,env));
            }
            thread::sleep(Duration::from_millis(150));
        }
        Err("Cannot authenticate to Movie Harbor. Enable remote control in the Mac app and check the access token.".into())
    }).await.map_err(|e|e.to_string())??;
    *state.0.lock().unwrap() = Some(client);
    Ok(env)
}
#[tauri::command]
pub fn remote_disconnect(state: tauri::State<RemoteClientState>) {
    state.0.lock().unwrap().take();
}
#[tauri::command]
pub async fn remote_call(
    action: String,
    mut payload: Value,
    state: tauri::State<'_, RemoteClientState>,
) -> Result<Value, String> {
    // Copy the connection details, so polling never holds a mutex over network I/O.
    let (port, token, local_root, remote_root) = {
        let slot = state.0.lock().unwrap();
        let c = slot.as_ref().ok_or("Not connected")?;
        (
            c.port,
            c.token.clone(),
            c.local_root.clone(),
            c.remote_root.clone(),
        )
    };
    tauri::async_runtime::spawn_blocking(move || {
        // Mapping does not own or terminate the SSH process.
        let map_path = |value: &str| -> Result<String, String> {
            let p = fs::canonicalize(value).map_err(|e| e.to_string())?;
            let relative = p
                .strip_prefix(&local_root)
                .map_err(|_| "Select files inside the configured shared folder")?;
            Ok(format!(
                "{}/{}",
                remote_root.trim_end_matches('/'),
                relative.to_string_lossy().replace('\\', "/")
            ))
        };
        if action == "inspect" {
            payload["path"] = json!(map_path(payload["path"].as_str().ok_or("Missing path")?)?);
        }
        if action == "start" {
            for item in payload["jobs"].as_array_mut().ok_or("Missing jobs")? {
                for key in ["source", "folder"] {
                    if let Some(s) = item["job"][key].as_str().filter(|s| !s.is_empty()) {
                        item["job"][key] = json!(map_path(s)?);
                    }
                }
            }
        }
        let mut value = rpc(port, &token, &action, payload)?;
        fn map_output(v: &mut Value, local: &Path, remote: &str) {
            match v {
                Value::Object(m) => {
                    for (k, v) in m {
                        if ["source", "folder", "output"].contains(&k.as_str()) {
                            if let Some(s) = v.as_str() {
                                if s == remote {
                                    *v = json!(local.to_string_lossy())
                                } else if let Some(relative) =
                                    s.strip_prefix(&format!("{}/", remote.trim_end_matches('/')))
                                {
                                    *v = json!(local.join(relative).to_string_lossy())
                                }
                            }
                        } else {
                            map_output(v, local, remote)
                        }
                    }
                }
                Value::Array(a) => {
                    for v in a {
                        map_output(v, local, remote)
                    }
                }
                _ => {}
            }
        }
        map_output(&mut value, &local_root, &remote_root);
        Ok(value)
    })
    .await
    .map_err(|e| e.to_string())?
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn authenticated_server_roundtrip_and_shutdown() {
        let root = fs::canonicalize(std::env::temp_dir()).unwrap();
        let q = Arc::new(Queue::default());
        let (session, info) = start_server(root, q.clone(), 0).unwrap();
        let port = info["port"].as_u64().unwrap() as u16;
        let token = info["token"].as_str().unwrap();
        assert_eq!(token.len(), 64);
        assert!(rpc(port, &"b".repeat(64), "status", json!({})).is_err());
        assert_eq!(
            rpc(port, token, "status", json!({})).unwrap()["running"],
            false
        );
        assert!(rpc(port, token, "execute", json!({"command":"anything"})).is_err());
        drop(session);
        thread::sleep(Duration::from_millis(150));
        assert!(TcpStream::connect(("127.0.0.1", port)).is_err());
    }
    #[cfg(unix)]
    #[test]
    fn symlinks_cannot_escape_shared_root() {
        let root = std::env::temp_dir().join(format!("harbor-symlink-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        let link = root.join("escape");
        std::os::unix::fs::symlink(root.parent().unwrap(), &link).unwrap();
        assert!(scoped_path(link.to_str().unwrap(), &root).is_err());
    }
    #[test]
    fn authentication_and_browser_requests() {
        let listener = TcpListener::bind(("127.0.0.1", 0)).unwrap();
        let address = listener.local_addr().unwrap();
        let token = "a".repeat(64);
        for (auth, extra, accepted) in [
            (token.clone(), "", true),
            ("b".repeat(64), "", false),
            (token.clone(), "Origin: https://example.com\r\n", false),
            (String::new(), "", false),
        ] {
            let expected = token.clone();
            let server = listener.try_clone().unwrap();
            let handle = thread::spawn(move || {
                let (mut s, _) = server.accept().unwrap();
                s.set_read_timeout(Some(Duration::from_secs(2))).unwrap();
                read_request(&mut s, &expected).is_ok()
            });
            let mut stream = TcpStream::connect(address).unwrap();
            write!(stream,"POST /rpc HTTP/1.1\r\nContent-Type: application/json\r\nAuthorization: Bearer {auth}\r\n{extra}Content-Length: 2\r\n\r\n{{}}").unwrap();
            assert_eq!(handle.join().unwrap(), accepted);
        }
    }
    #[test]
    fn paths_cannot_escape_shared_root() {
        let root = std::env::temp_dir().join(format!("harbor-scope-{}", std::process::id()));
        fs::create_dir_all(&root).unwrap();
        let root = fs::canonicalize(root).unwrap();
        assert!(scoped_path(root.to_str().unwrap(), &root).is_ok());
        assert!(scoped_path(root.parent().unwrap().to_str().unwrap(), &root).is_err());
    }
    #[test]
    fn ssh_fields_reject_options_and_shell_syntax() {
        let mut c = Connection {
            host: "mac.local".into(),
            user: "user".into(),
            token: "a".repeat(64),
            local_root: String::new(),
            key_path: String::new(),
        };
        assert!(valid_connection(&c));
        c.host = "-oProxyCommand=bad".into();
        assert!(!valid_connection(&c));
        c.host = "mac.local;command".into();
        assert!(!valid_connection(&c));
    }
}
