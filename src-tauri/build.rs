use std::path::{Path, PathBuf};
use std::process::Command;
use std::time::SystemTime;

fn main() {
    publish_lhm_sidecar();
    tauri_build::build();
}

fn publish_lhm_sidecar() {
    let target = std::env::var("TARGET").unwrap_or_default();
    if !target.contains("windows") {
        return;
    }

    let manifest_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let helper_dir = manifest_dir.join("..").join("sensor-helper");
    let project = helper_dir.join("StmLhm.csproj");
    let program = helper_dir.join("Program.cs");
    let out_dir = manifest_dir.join("binaries").join("_publish");
    let dest = manifest_dir.join("binaries").join(format!("stm-lhm-{target}.exe"));

    println!("cargo:rerun-if-changed={}", project.display());
    println!("cargo:rerun-if-changed={}", program.display());

    if dest.is_file() && newest(&[&project, &program]) <= file_time(&dest) {
        return;
    }

    std::fs::create_dir_all(&out_dir).expect("create sidecar publish dir");
    let status = Command::new("dotnet")
        .args([
            "publish",
            project.to_str().expect("helper path"),
            "-c",
            "Release",
            "-r",
            "win-x64",
            "--self-contained",
            "true",
            "-p:PublishSingleFile=true",
            "-p:IncludeNativeLibrariesForSelfExtract=true",
            "-p:EnableCompressionInSingleFile=true",
            "-o",
            out_dir.to_str().expect("publish dir"),
        ])
        .status()
        .expect("dotnet publish failed to start — install the .NET 8 SDK");
    assert!(status.success(), "dotnet publish of stm-lhm failed");

    let published = out_dir.join("stm-lhm.exe");
    std::fs::create_dir_all(dest.parent().unwrap()).expect("create binaries dir");
    std::fs::copy(&published, &dest)
        .unwrap_or_else(|e| panic!("copy sidecar {} -> {}: {e}", published.display(), dest.display()));
}

fn file_time(path: &Path) -> SystemTime {
    path.metadata()
        .and_then(|m| m.modified())
        .unwrap_or(SystemTime::UNIX_EPOCH)
}

fn newest(paths: &[&Path]) -> SystemTime {
    paths.iter().map(|p| file_time(p)).max().unwrap_or(SystemTime::UNIX_EPOCH)
}
