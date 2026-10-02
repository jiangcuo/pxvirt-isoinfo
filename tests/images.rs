//! Build small images with genisoimage and check the detection result.

use std::path::{Path, PathBuf};
use std::process::Command;

use serde_json::Value;

struct TempDir(PathBuf);

impl TempDir {
    fn new(name: &str) -> Self {
        let path = std::env::temp_dir().join(format!("pxvirt-isoinfo-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&path);
        std::fs::create_dir_all(&path).unwrap();
        Self(path)
    }

    fn file(&self, path: &str, data: &[u8]) {
        let path = self.0.join("tree").join(path);
        std::fs::create_dir_all(path.parent().unwrap()).unwrap();
        std::fs::write(path, data).unwrap();
    }
}

impl Drop for TempDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn have_genisoimage() -> bool {
    let found = Command::new("genisoimage").arg("-version").output().is_ok();
    if !found {
        eprintln!("genisoimage not found, skipping");
    }
    found
}

fn mkiso(dir: &TempDir, args: &[&str]) -> PathBuf {
    let iso = dir.0.join("test.iso");
    let status = Command::new("genisoimage")
        .arg("-quiet")
        .args(args)
        .arg("-o")
        .arg(&iso)
        .arg(dir.0.join("tree"))
        .status()
        .unwrap();
    assert!(status.success());
    iso
}

fn isoinfo(iso: &Path) -> Value {
    let out = Command::new(env!("CARGO_BIN_EXE_pxvirt-isoinfo")).arg(iso).output().unwrap();
    assert!(out.status.success(), "{}", String::from_utf8_lossy(&out.stderr));
    serde_json::from_slice(&out.stdout).unwrap()
}

fn wim(xml: &str) -> Vec<u8> {
    let mut data = vec![0xff, 0xfe];
    data.extend(xml.encode_utf16().flat_map(|u| u.to_le_bytes()));

    let offset = 4096u64;
    let mut wim = vec![0u8; offset as usize];
    wim[0..8].copy_from_slice(b"MSWIM\0\0\0");
    wim[72..80].copy_from_slice(&(data.len() as u64).to_le_bytes());
    wim[80..88].copy_from_slice(&offset.to_le_bytes());
    wim.extend(data);
    wim
}

const WIN_XML: &str = r#"<WIM>
  <IMAGE INDEX="1"><NAME>Windows Server 2025 Standard</NAME>
    <WINDOWS><ARCH>12</ARCH><EDITIONID>ServerStandard</EDITIONID><INSTALLATIONTYPE>Server Core</INSTALLATIONTYPE>
    <VERSION><MAJOR>10</MAJOR><MINOR>0</MINOR><BUILD>26100</BUILD></VERSION></WINDOWS></IMAGE>
  <IMAGE INDEX="2"><NAME>Windows Server 2025 Standard (Desktop Experience)</NAME>
    <WINDOWS><ARCH>12</ARCH><EDITIONID>ServerStandard</EDITIONID><INSTALLATIONTYPE>Server</INSTALLATIONTYPE>
    <VERSION><MAJOR>10</MAJOR><MINOR>0</MINOR><BUILD>26100</BUILD></VERSION></WINDOWS></IMAGE>
</WIM>"#;

fn check_windows(info: &Value) {
    assert_eq!(info["type"], "windows");
    assert_eq!(info["name"], "Windows Server 2025");
    assert_eq!(info["ostype"], "win11");
    assert_eq!(info["arch"], "aarch64");
    assert_eq!(info["installer"], "windows");
    let images = info["images"].as_array().unwrap();
    assert_eq!(images.len(), 2);
    assert_eq!(images[1]["index"], 2);
    assert_eq!(images[1]["name"], "Windows Server 2025 Standard (Desktop Experience)");
}

#[test]
fn windows_udf() {
    if !have_genisoimage() {
        return;
    }
    let dir = TempDir::new("windows-udf");
    dir.file("sources/install.wim", &wim(WIN_XML));
    let info = isoinfo(&mkiso(&dir, &["-udf", "-V", "SSS_ARM64FRE_EN-US_DV9"]));
    assert_eq!(info["filesystem"], "udf");
    assert_eq!(info["label"], "SSS_ARM64FRE_EN-US_DV9");
    check_windows(&info);
}

#[test]
fn windows_joliet() {
    if !have_genisoimage() {
        return;
    }
    let dir = TempDir::new("windows-joliet");
    dir.file("sources/install.esd", &wim(WIN_XML));
    let info = isoinfo(&mkiso(&dir, &["-J", "-R"]));
    assert_eq!(info["filesystem"], "iso9660");
    check_windows(&info);
}

#[test]
fn ubuntu() {
    if !have_genisoimage() {
        return;
    }
    let dir = TempDir::new("ubuntu");
    dir.file(".disk/info", b"Ubuntu-Server 24.04.1 LTS \"Noble Numbat\" - Release arm64 (20240827)\n");
    dir.file("casper/vmlinuz", b"");
    let info = isoinfo(&mkiso(&dir, &["-R"]));
    assert_eq!(info["id"], "ubuntu");
    assert_eq!(info["version"], "24.04.1");
    assert_eq!(info["arch"], "aarch64");
    assert_eq!(info["installer"], "ubuntu");
}

#[test]
fn debian() {
    if !have_genisoimage() {
        return;
    }
    let dir = TempDir::new("debian");
    dir.file(
        ".disk/info",
        b"Debian GNU/Linux 12.7.0 \"Bookworm\" - Official amd64 NETINST with firmware 20240831-10:38",
    );
    let info = isoinfo(&mkiso(&dir, &["-R"]));
    assert_eq!(info["id"], "debian");
    assert_eq!(info["version"], "12.7.0");
    assert_eq!(info["arch"], "x86_64");
    assert!(info.get("installer").is_none());
}

#[test]
fn anaconda() {
    if !have_genisoimage() {
        return;
    }
    let dir = TempDir::new("anaconda");
    dir.file(".treeinfo", b"[general]\narch = x86_64\nfamily = Example Linux\nversion = 1.2\n");
    let info = isoinfo(&mkiso(&dir, &["-J", "-R"]));
    assert_eq!(info["id"], "anaconda");
    assert_eq!(info["name"], "Example Linux");
    assert_eq!(info["version"], "1.2");
    assert_eq!(info["installer"], "kickstart");
}

#[test]
fn unknown() {
    if !have_genisoimage() {
        return;
    }
    let dir = TempDir::new("unknown");
    dir.file("README.TXT", b"hello");
    let info = isoinfo(&mkiso(&dir, &["-V", "DATA"]));
    assert_eq!(info["type"], "unknown");
    assert_eq!(info["label"], "DATA");
}
