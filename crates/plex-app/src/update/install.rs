//! Putting a verified release package in place, per kind of install.
//!
//! * **AppImage**: the new file is written next to the running one and renamed
//!   over it. The running process keeps the old file open, so nothing it uses
//!   goes away under it.
//! * **Windows**: Windows lets a running program's files be renamed but not
//!   overwritten. The installer case moves this install's programs and
//!   libraries aside to `*.old` and runs the new installer silently, while the
//!   app keeps running; the zip case moves each file it replaces aside and
//!   copies the new one in. Either way a failure puts the old files back, and
//!   the next start removes the `*.old` files.
//! * **macOS**: the disk image is attached, its app copied beside this one and
//!   swapped in by renaming.

use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::time::Duration;

use super::{agent, listed_digest, Asset, Kind, MAX_PACKAGE_BYTES};

/// How far apart download progress is reported.
const PROGRESS_STEP: u64 = 512 * 1024;
/// The longest one download may take.
const DOWNLOAD_TIME: Duration = Duration::from_secs(2 * 60 * 60);
/// The AppImage's download, beside the AppImage it replaces.
const APPIMAGE_PART: &str = ".chewvy-update.part";

/// How this process was installed.
pub(super) fn detect() -> Kind {
    let exe = std::env::current_exe().ok();
    detect_from(
        exe.as_deref(),
        std::env::var_os("APPIMAGE").map(PathBuf::from),
    )
}

/// [`detect`] for a given executable path and `$APPIMAGE`.
pub(super) fn detect_from(exe: Option<&Path>, appimage: Option<PathBuf>) -> Kind {
    if cfg!(target_os = "linux") {
        return match appimage.filter(|path| path.is_file()) {
            Some(path) if cfg!(target_arch = "x86_64") => Kind::AppImage(path),
            Some(_) => Kind::Unmanaged("Only the x86_64 AppImage is published."),
            None => Kind::Unmanaged(
                "This copy was not started from the AppImage, so whatever installed it updates it.",
            ),
        };
    }
    let Some(dir) = exe.and_then(Path::parent) else {
        return Kind::Unmanaged("The app could not find where it is installed.");
    };
    if cfg!(windows) {
        if dir.join("uninstall.exe").is_file() {
            return Kind::Installer(dir.to_owned());
        }
        if dir.join("libmpv-2.dll").is_file() {
            return Kind::Portable(dir.to_owned());
        }
        return Kind::Unmanaged("This copy is not one the installer or the zip set up.");
    }
    if cfg!(target_os = "macos") {
        let bundle = dir
            .ancestors()
            .nth(2)
            .filter(|path| path.extension().is_some_and(|ext| ext == "app"));
        return match bundle {
            Some(bundle) if cfg!(target_arch = "aarch64") => Kind::Bundle(bundle.to_owned()),
            Some(_) => Kind::Unmanaged("Only the Apple Silicon disk image is published."),
            None => Kind::Unmanaged("This copy is not inside an app bundle."),
        };
    }
    Kind::Unmanaged("No package is published for this system.")
}

/// Reports an install step and, where it can say, how far through it is.
pub(super) type Stage<'a> = &'a dyn Fn(&'static str, Option<f32>);

/// Download `package`, check it against `sums` and put it in place.
/// `progress` hears the bytes downloaded; `stage` each install step after.
pub(super) fn run(
    kind: &Kind,
    package: &Asset,
    sums: &Asset,
    progress: &dyn Fn(u64),
    stage: Stage<'_>,
) -> Result<(), String> {
    let listed = fetch_sums(sums)?;
    let want = listed_digest(&listed, &package.name)
        .ok_or_else(|| format!("SHA256SUMS does not list {}", package.name))?;
    let dest = staging_path(kind, &package.name)?;
    let got = download(&package.url, &dest, progress);
    let got = match got {
        Ok(digest) => digest,
        Err(why) => {
            let _ = std::fs::remove_file(&dest);
            return Err(why);
        }
    };
    if got != want {
        let _ = std::fs::remove_file(&dest);
        return Err("The download did not match its checksum, so nothing was installed.".into());
    }
    stage("Checking the download", Some(1.0));
    place(kind, &dest, stage).inspect_err(|_| {
        let _ = std::fs::remove_file(&dest);
    })
}

fn fetch_sums(sums: &Asset) -> Result<String, String> {
    let mut answer = agent(Some(Duration::from_secs(60)))
        .get(&sums.url)
        .call()
        .map_err(|_| "Could not download the release's checksums".to_owned())?;
    answer
        .body_mut()
        .with_config()
        .limit(1024 * 1024)
        .read_to_string()
        .map_err(|_| "The release's checksums could not be read".to_owned())
}

/// Where the package is downloaded to: beside an AppImage, so the rename that
/// replaces it stays on one filesystem; the temp folder for everything else.
fn staging_path(kind: &Kind, name: &str) -> Result<PathBuf, String> {
    if let Kind::AppImage(target) = kind {
        let dir = target
            .parent()
            .ok_or("The AppImage has no folder to update it in")?;
        return Ok(dir.join(APPIMAGE_PART));
    }
    let dir = temp_dir();
    std::fs::create_dir_all(&dir)
        .map_err(|e| format!("Could not create {}: {e}", dir.display()))?;
    Ok(dir.join(name))
}

fn temp_dir() -> PathBuf {
    std::env::temp_dir().join("chewvy-update")
}

/// Stream `url` into `dest`, answering the SHA-256 of what was written.
fn download(url: &str, dest: &Path, progress: &dyn Fn(u64)) -> Result<String, String> {
    let mut answer = agent(Some(DOWNLOAD_TIME))
        .get(url)
        .call()
        .map_err(|_| "Could not download the update".to_owned())?;
    let mut reader = answer
        .body_mut()
        .with_config()
        .limit(MAX_PACKAGE_BYTES)
        .reader();
    let mut file = std::fs::File::create(dest)
        .map_err(|e| format!("Could not write {}: {e}", dest.display()))?;
    let mut digest = ring::digest::Context::new(&ring::digest::SHA256);
    let mut buf = vec![0_u8; 64 * 1024];
    let (mut done, mut reported) = (0_u64, 0_u64);
    loop {
        let read = reader
            .read(&mut buf)
            .map_err(|_| "The download was cut short".to_owned())?;
        let Some(chunk) = buf.get(..read).filter(|chunk| !chunk.is_empty()) else {
            break;
        };
        digest.update(chunk);
        file.write_all(chunk)
            .map_err(|e| format!("Could not write the update: {e}"))?;
        done += chunk.len() as u64;
        if done - reported >= PROGRESS_STEP {
            progress(done);
            reported = done;
        }
    }
    progress(done);
    file.sync_all()
        .map_err(|e| format!("Could not write the update: {e}"))?;
    Ok(hex(digest.finish().as_ref()))
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|byte| format!("{byte:02x}")).collect()
}

/// Put the verified file at `staged` in place for `kind`.
fn place(kind: &Kind, staged: &Path, stage: Stage<'_>) -> Result<(), String> {
    let placed = match kind {
        Kind::AppImage(target) => {
            stage("Replacing the AppImage", None);
            replace_appimage(staged, target)
        }
        Kind::Installer(dir) => run_installer(staged, dir, stage),
        Kind::Portable(dir) => replace_portable(staged, dir, stage),
        Kind::Bundle(bundle) => replace_bundle(staged, bundle, stage),
        Kind::Unmanaged(why) => Err((*why).to_owned()),
    };
    if placed.is_ok() {
        stage("Preparing the new version", None);
        warm_up(kind);
        stage("Finished", Some(1.0));
    }
    placed
}

/// The longest the first start of the new version may take before the
/// install stops waiting for it.
const WARM_UP: Duration = Duration::from_secs(90);

/// Start the new version once with `--help`, which prints and exits. The first
/// start of a new program is when Windows Defender (and macOS's own check)
/// scans it, ten seconds or more for a file this size; doing it here, under
/// the progress bar, is what lets "Restart now" open the new version at once.
fn warm_up(kind: &Kind) {
    let exe = match kind {
        Kind::AppImage(path) => path.clone(),
        Kind::Installer(dir) | Kind::Portable(dir) => dir.join("chewvy.exe"),
        Kind::Bundle(bundle) => bundle.join("Contents").join("MacOS").join("chewvy"),
        Kind::Unmanaged(_) => return,
    };
    let mut cmd = std::process::Command::new(&exe);
    cmd.arg("--help")
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null());
    hide_console(&mut cmd);
    let Ok(mut child) = cmd.spawn() else {
        log::warn!("update: could not start {} to prepare it", exe.display());
        return;
    };
    let started = std::time::Instant::now();
    while started.elapsed() < WARM_UP {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    log::warn!("update: the new version's first start ended with {status}");
                }
                return;
            }
            Ok(None) => std::thread::sleep(Duration::from_millis(100)),
            Err(_) => break,
        }
    }
    let _ = child.kill();
    let _ = child.wait();
}

fn replace_appimage(staged: &Path, target: &Path) -> Result<(), String> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        std::fs::set_permissions(staged, std::fs::Permissions::from_mode(0o755))
            .map_err(|e| format!("Could not mark the update executable: {e}"))?;
    }
    std::fs::rename(staged, target)
        .map_err(|e| format!("Could not replace {}: {e}", target.display()))
}

/// Unpack the zip with the `tar` Windows ships, then move each of its files
/// in, keeping the one it replaces as `*.old` until the next start.
fn replace_portable(zip: &Path, dir: &Path, stage: Stage<'_>) -> Result<(), String> {
    stage("Unpacking the update", None);
    let unpacked = temp_dir().join("unpacked");
    let _ = std::fs::remove_dir_all(&unpacked);
    std::fs::create_dir_all(&unpacked).map_err(|e| format!("Could not unpack the update: {e}"))?;
    let tar = std::env::var_os("SystemRoot")
        .map(|root| PathBuf::from(root).join("System32").join("tar.exe"))
        .unwrap_or_else(|| PathBuf::from("tar"));
    let mut unzip = std::process::Command::new(tar);
    unzip.arg("-xf").arg(zip).arg("-C").arg(&unpacked);
    hide_console(&mut unzip);
    command(&mut unzip, "unpack the update")?;
    let mut moved = Vec::new();
    let result = (|| {
        let entries: Vec<_> = std::fs::read_dir(&unpacked)
            .map_err(|e| format!("Could not read the update: {e}"))?
            .flatten()
            .collect();
        let count = entries.len().max(1) as f32;
        for (copied, entry) in entries.into_iter().enumerate() {
            stage("Copying the new files", Some(copied as f32 / count));
            let dest = dir.join(entry.file_name());
            if dest.exists() {
                moved.push(move_aside(&dest)?);
            }
            std::fs::copy(entry.path(), &dest)
                .map_err(|e| format!("Could not write {}: {e}", dest.display()))?;
        }
        Ok(())
    })();
    if result.is_err() {
        put_back(moved);
    }
    let _ = std::fs::remove_dir_all(&unpacked);
    let _ = std::fs::remove_file(zip);
    result
}

/// Run the downloaded installer silently over this install while the app
/// keeps running: its programs and libraries are moved aside first, so the
/// installer finds nothing in use. The new version starts on the next start.
fn run_installer(setup: &Path, dir: &Path, stage: Stage<'_>) -> Result<(), String> {
    stage("Moving the old version aside", None);
    let entries =
        std::fs::read_dir(dir).map_err(|e| format!("Could not read {}: {e}", dir.display()))?;
    let mut moved = Vec::new();
    let result = (|| {
        for entry in entries.flatten() {
            let path = entry.path();
            let running = path.extension().is_some_and(|ext| {
                ext.eq_ignore_ascii_case("exe") || ext.eq_ignore_ascii_case("dll")
            });
            if running && path.is_file() {
                moved.push(move_aside(&path)?);
            }
        }
        // A silent installer reports nothing while it runs.
        stage("Running the installer", None);
        let mut install = std::process::Command::new(setup);
        install.arg("/S");
        install_dir_arg(&mut install, dir);
        command(&mut install, "run the installer")?;
        if dir.join("chewvy.exe").is_file() {
            Ok(())
        } else {
            Err("The installer finished without installing Chewvy".to_owned())
        }
    })();
    match result {
        // A file the installer did not write again (a library the user put
        // beside the program) comes back as it was.
        Ok(()) => {
            for (path, old) in &moved {
                if !path.exists() {
                    let _ = std::fs::rename(old, path);
                }
            }
        }
        Err(_) => put_back(moved),
    }
    let _ = std::fs::remove_file(setup);
    result
}

/// Point the installer at this install's folder. NSIS reads `/D=` only as
/// the last argument and unquoted, even when the path has spaces in it.
fn install_dir_arg(cmd: &mut std::process::Command, dir: &Path) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        cmd.raw_arg(format!("/D={}", dir.display()));
    }
    #[cfg(not(windows))]
    let _ = (cmd, dir);
}

/// Rename `path` to `path.old`, which Windows allows for a file in use.
/// Answers both, for [`put_back`].
fn move_aside(path: &Path) -> Result<(PathBuf, PathBuf), String> {
    let mut old = path.as_os_str().to_owned();
    old.push(".old");
    let old = PathBuf::from(old);
    let _ = std::fs::remove_file(&old);
    std::fs::rename(path, &old)
        .map_err(|e| format!("Could not move {} aside: {e}", path.display()))?;
    Ok((path.to_owned(), old))
}

/// Undo [`move_aside`], so a half-done update still starts as before.
fn put_back(moved: Vec<(PathBuf, PathBuf)>) {
    for (path, old) in moved.into_iter().rev() {
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::rename(&old, &path);
    }
}

/// Copy the app out of the disk image beside `bundle`, then swap the two.
fn replace_bundle(dmg: &Path, bundle: &Path, stage: Stage<'_>) -> Result<(), String> {
    stage("Opening the disk image", None);
    let parent = bundle
        .parent()
        .ok_or("The app has no folder to update it in")?;
    let mount = temp_dir().join("mount");
    std::fs::create_dir_all(&mount).map_err(|e| format!("Could not open the update: {e}"))?;
    command(
        std::process::Command::new("hdiutil")
            .args([
                "attach",
                "-nobrowse",
                "-readonly",
                "-noautoopen",
                "-mountpoint",
            ])
            .arg(&mount)
            .arg(dmg),
        "open the disk image",
    )?;
    let fresh = parent.join(".Chewvy-update.app");
    let copied = (|| {
        let source = if mount.join("Chewvy.app").is_dir() {
            mount.join("Chewvy.app")
        } else if mount.join("Contents").join("MacOS").is_dir() {
            mount.clone()
        } else {
            return Err("The disk image holds no app".to_owned());
        };
        let _ = std::fs::remove_dir_all(&fresh);
        stage("Copying the new app", None);
        command(
            std::process::Command::new("ditto").arg(&source).arg(&fresh),
            "copy the new app",
        )
    })();
    let _ = std::process::Command::new("hdiutil")
        .arg("detach")
        .arg(&mount)
        .status();
    let _ = std::fs::remove_file(dmg);
    if let Err(why) = copied {
        let _ = std::fs::remove_dir_all(&fresh);
        return Err(why);
    }
    stage("Swapping in the new app", None);
    let old = parent.join(".Chewvy-old.app");
    let _ = std::fs::remove_dir_all(&old);
    std::fs::rename(bundle, &old)
        .map_err(|e| format!("Could not replace {}: {e}", bundle.display()))?;
    if let Err(e) = std::fs::rename(&fresh, bundle) {
        let _ = std::fs::rename(&old, bundle);
        return Err(format!("Could not replace {}: {e}", bundle.display()));
    }
    let _ = std::fs::remove_dir_all(&old);
    Ok(())
}

/// Run `cmd` to its end; `what` names it for the error.
fn command(cmd: &mut std::process::Command, what: &str) -> Result<(), String> {
    match cmd.status() {
        Ok(status) if status.success() => Ok(()),
        Ok(status) => Err(format!("Could not {what} ({status})")),
        Err(e) => Err(format!("Could not {what}: {e}")),
    }
}

/// Keep a console window from flashing up for a helper program on Windows.
fn hide_console(cmd: &mut std::process::Command) {
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt as _;
        const CREATE_NO_WINDOW: u32 = 0x0800_0000;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    #[cfg(not(windows))]
    let _ = cmd;
}

/// Start the installed version of `kind`.
pub(super) fn relaunch(kind: &Kind) -> Result<(), std::io::Error> {
    let mut cmd = match kind {
        Kind::AppImage(path) => {
            let mut cmd = std::process::Command::new(path);
            // The old AppImage's mount goes away with this process; the new
            // one sets up its own.
            for name in ["APPIMAGE", "APPDIR", "ARGV0", "OWD"] {
                cmd.env_remove(name);
            }
            cmd
        }
        Kind::Installer(dir) | Kind::Portable(dir) => {
            std::process::Command::new(dir.join("chewvy.exe"))
        }
        Kind::Bundle(bundle) => {
            let mut cmd = std::process::Command::new("open");
            cmd.arg("-n").arg(bundle);
            cmd
        }
        Kind::Unmanaged(_) => return Ok(()),
    };
    cmd.spawn().map(drop)
}

/// Remove what an update of `kind` left behind.
pub(super) fn tidy(kind: &Kind) {
    match kind {
        Kind::AppImage(path) => {
            if let Some(dir) = path.parent() {
                let _ = std::fs::remove_file(dir.join(APPIMAGE_PART));
            }
        }
        Kind::Installer(dir) | Kind::Portable(dir) => {
            let Ok(entries) = std::fs::read_dir(dir) else {
                return;
            };
            for entry in entries.flatten() {
                if entry.path().extension().is_some_and(|ext| ext == "old") {
                    let _ = std::fs::remove_file(entry.path());
                }
            }
        }
        Kind::Bundle(bundle) => {
            if let Some(parent) = bundle.parent() {
                let _ = std::fs::remove_dir_all(parent.join(".Chewvy-old.app"));
                let _ = std::fs::remove_dir_all(parent.join(".Chewvy-update.app"));
            }
        }
        Kind::Unmanaged(_) => {}
    }
    let _ = std::fs::remove_dir_all(temp_dir());
}
