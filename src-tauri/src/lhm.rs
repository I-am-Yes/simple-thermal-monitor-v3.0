//! LibreHardwareMonitor 0.9.6 sidecar (PawnIO). Never WinRing0.
//! The helper can stall on CPU Open when elevated; this worker never blocks the UI.

use crate::temps::{plausible, round_tenth, ThermalReading};
use serde::Deserialize;
use std::io::{BufRead, BufReader, Write};
use std::path::PathBuf;
use std::process::{Child, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Debug, Deserialize)]
#[serde(rename_all = "camelCase")]
struct LhmLine {
    cpu_package: Option<f32>,
    p_core_0: Option<f32>,
    gpu: Option<f32>,
    ssd: Option<f32>,
}

struct Helper {
    child: Child,
    stdin: ChildStdin,
    stdout: BufReader<ChildStdout>,
}

static LAST: Mutex<Option<ThermalReading>> = Mutex::new(None);
static STARTED: AtomicBool = AtomicBool::new(false);

pub fn warmup() {
    start_worker();
}

pub fn latest() -> Option<ThermalReading> {
    start_worker();
    LAST.lock().ok().and_then(|guard| guard.clone())
}

fn start_worker() {
    if STARTED.swap(true, Ordering::SeqCst) {
        return;
    }
    let _ = std::thread::Builder::new()
        .name("lhm-worker".into())
        .spawn(worker);
}

fn worker() {
    let mut backoff = Duration::from_secs(1);
    loop {
        match spawn() {
            Some(mut helper) => {
                let mut first = true;
                loop {
                    let timeout = if first {
                        Duration::from_secs(8)
                    } else {
                        Duration::from_secs(2)
                    };
                    match ping(&mut helper, timeout) {
                        Some(reading) => {
                            if let Ok(mut last) = LAST.lock() {
                                *last = Some(reading);
                            }
                            first = false;
                            backoff = Duration::from_secs(1);
                            std::thread::sleep(Duration::from_millis(400));
                        }
                        None => {
                            let _ = helper.child.kill();
                            let _ = helper.child.wait();
                            break;
                        }
                    }
                }
            }
            None => std::thread::sleep(backoff),
        }
        std::thread::sleep(backoff);
        backoff = (backoff * 2).min(Duration::from_secs(20));
    }
}

fn ping(helper: &mut Helper, timeout: Duration) -> Option<ThermalReading> {
    if helper.stdin.write_all(b"read\n").is_err() || helper.stdin.flush().is_err() {
        return None;
    }

    let cancel = Arc::new(AtomicBool::new(false));
    let pid = helper.child.id();
    let watchdog = cancel.clone();
    let _ = std::thread::Builder::new()
        .name("lhm-watchdog".into())
        .spawn(move || {
            let deadline = Instant::now() + timeout;
            while Instant::now() < deadline {
                if watchdog.load(Ordering::Relaxed) {
                    return;
                }
                std::thread::sleep(Duration::from_millis(40));
            }
            if !watchdog.load(Ordering::Relaxed) {
                terminate_pid(pid);
            }
        });

    let parsed = read_json_line(helper);
    cancel.store(true, Ordering::Relaxed);
    parsed.map(into_reading)
}

fn into_reading(line: LhmLine) -> ThermalReading {
    ThermalReading {
        cpu_package: sanitize(line.cpu_package),
        p_core_0: sanitize(line.p_core_0),
        gpu: sanitize(line.gpu),
        ssd: sanitize(line.ssd),
    }
}

fn sanitize(temp: Option<f32>) -> Option<f32> {
    temp.filter(|t| plausible(*t)).map(round_tenth)
}

fn spawn() -> Option<Helper> {
    let path = sidecar_path()?;
    let mut command = Command::new(&path);
    command
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .current_dir(path.parent().unwrap_or_else(|| std::path::Path::new(".")));

    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        command.creation_flags(CREATE_NO_WINDOW);
    }

    let mut child = command.spawn().ok()?;
    let stdin = child.stdin.take()?;
    let stdout = BufReader::new(child.stdout.take()?);
    Some(Helper {
        child,
        stdin,
        stdout,
    })
}

fn read_json_line(helper: &mut Helper) -> Option<LhmLine> {
    let mut line = String::new();
    loop {
        line.clear();
        match helper.stdout.read_line(&mut line) {
            Ok(0) => return None,
            Ok(_) => {
                let trimmed = line.trim();
                if !trimmed.starts_with('{') {
                    continue;
                }
                if let Ok(parsed) = serde_json::from_str::<LhmLine>(trimmed) {
                    return Some(parsed);
                }
            }
            Err(_) => return None,
        }
    }
}

fn terminate_pid(pid: u32) {
    #[cfg(windows)]
    {
        use windows::Win32::Foundation::CloseHandle;
        use windows::Win32::System::Threading::{
            OpenProcess, TerminateProcess, PROCESS_TERMINATE,
        };
        unsafe {
            if let Ok(handle) = OpenProcess(PROCESS_TERMINATE, false, pid) {
                let _ = TerminateProcess(handle, 1);
                let _ = CloseHandle(handle);
            }
        }
    }
    #[cfg(not(windows))]
    {
        let _ = pid;
    }
}

fn sidecar_path() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let exe_dir = exe.parent()?;
    let names = [
        "stm-lhm.exe",
        "stm-lhm-x86_64-pc-windows-msvc.exe",
        "stm-lhm-aarch64-pc-windows-msvc.exe",
    ];

    let mut dirs = vec![exe_dir.to_path_buf()];
    if let Some(parent) = exe_dir.parent() {
        dirs.push(parent.to_path_buf());
    }
    dirs.push(PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("binaries"));
    dirs.push(
        PathBuf::from(env!("CARGO_MANIFEST_DIR"))
            .join("binaries")
            .join("_publish"),
    );

    for dir in dirs {
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Some(candidate);
            }
        }
    }
    None
}
