//! The updater's reading of a release: versions, the package an install takes,
//! and the checksum file.

use std::path::PathBuf;

use super::*;

fn appimage() -> Kind {
    Kind::AppImage(PathBuf::from("/home/user/.local/bin/chewvy"))
}

/// A `releases/latest` answer with the files a release workflow attaches.
fn answer(tag: &str) -> String {
    let version = tag.trim_start_matches('v');
    let asset = |name: String| {
        format!(
            r#"{{"name":"{name}","browser_download_url":"https://github.com/o/r/releases/download/{tag}/{name}","size":1234}}"#
        )
    };
    let assets = [
        format!("Chewvy-{version}-x86_64.AppImage"),
        format!("Chewvy-{version}-windows-x86_64-setup.exe"),
        format!("Chewvy-{version}-windows-x86_64.zip"),
        "SHA256SUMS".to_owned(),
    ]
    .into_iter()
    .map(asset)
    .collect::<Vec<_>>()
    .join(",");
    format!(
        r#"{{"tag_name":"{tag}","draft":false,"prerelease":false,"body":" Fixes. ","assets":[{assets}]}}"#
    )
}

#[test]
fn versions_compare_by_number_and_a_pre_release_comes_first() {
    let v = |text| Version::parse(text).unwrap_or_else(|| panic!("{text} parses"));
    assert!(v("0.2.1") > v("0.2.0"));
    assert!(v("0.10.0") > v("0.9.9"));
    assert!(v("v1.0.0") > v("0.99.0"));
    assert!(v("0.3.0") > v("0.3.0-rc1"));
    assert!(v("0.3.0-rc1") > v("0.2.9"));
    assert_eq!(v("v0.2"), v("0.2.0"));
    assert_eq!(Version::parse("latest"), None);
    assert_eq!(Version::parse("1.2.3.4"), None);
}

#[test]
fn a_newer_release_offers_the_package_this_install_came_from() {
    let release = parse_latest(answer("v0.3.0").as_bytes(), "0.2.0", &appimage())
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("0.3.0 is newer"));
    assert_eq!(release.version, "0.3.0");
    assert_eq!(release.notes, "Fixes.");
    let package = release
        .package
        .unwrap_or_else(|| panic!("an AppImage is attached"));
    assert_eq!(package.name, "Chewvy-0.3.0-x86_64.AppImage");
    assert!(package.url.starts_with("https://github.com/"));
    assert_eq!(release.sums.map(|s| s.name).as_deref(), Some("SHA256SUMS"));

    let windows = Kind::Installer(PathBuf::from("C:/Chewvy"));
    let release = parse_latest(answer("v0.3.0").as_bytes(), "0.2.0", &windows)
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("0.3.0 is newer"));
    assert_eq!(
        release.package.map(|p| p.name).as_deref(),
        Some("Chewvy-0.3.0-windows-x86_64-setup.exe")
    );
}

#[test]
fn the_same_or_an_older_release_is_not_offered() {
    for tag in ["v0.2.0", "v0.1.9"] {
        let found = parse_latest(answer(tag).as_bytes(), "0.2.0", &appimage());
        assert_eq!(found, Ok(None), "{tag}");
    }
}

#[test]
fn a_release_with_no_package_for_this_install_names_none() {
    // No disk image is attached until the macOS workflow has run.
    let mac = Kind::Bundle(PathBuf::from("/Applications/Chewvy.app"));
    let release = parse_latest(answer("v0.3.0").as_bytes(), "0.2.0", &mac)
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("0.3.0 is newer"));
    assert_eq!(release.package, None);
    let unmanaged = Kind::Unmanaged("a development build");
    let release = parse_latest(answer("v0.3.0").as_bytes(), "0.2.0", &unmanaged)
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("0.3.0 is newer"));
    assert_eq!(release.package, None);
}

#[test]
fn an_answer_that_is_not_a_release_is_an_error() {
    assert!(parse_latest(b"{\"message\":\"Not Found\"}", "0.2.0", &appimage()).is_err());
    assert!(parse_latest(b"<html>", "0.2.0", &appimage()).is_err());
}

#[test]
fn the_listed_digest_is_the_one_for_that_exact_file() {
    let a = "a".repeat(64);
    let b = "B".repeat(64);
    let sums = format!(
        "{a}  Chewvy-0.3.0-x86_64.AppImage\n{b} *Chewvy-0.3.0-windows-x86_64.zip\nnot-a-digest  SHA256SUMS\n"
    );
    assert_eq!(
        listed_digest(&sums, "Chewvy-0.3.0-x86_64.AppImage").as_deref(),
        Some(a.as_str())
    );
    assert_eq!(
        listed_digest(&sums, "Chewvy-0.3.0-windows-x86_64.zip"),
        Some("b".repeat(64))
    );
    assert_eq!(listed_digest(&sums, "Chewvy-0.3.0"), None);
    assert_eq!(listed_digest(&sums, "SHA256SUMS"), None);
}

#[test]
fn an_offer_says_why_this_install_cannot_take_it() {
    let mut updater = Updater {
        kind: Kind::Unmanaged("a development build"),
        ..Updater::default()
    };
    let release = parse_latest(answer("v0.3.0").as_bytes(), "0.2.0", &updater.kind)
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("0.3.0 is newer"));
    updater.apply(Event::Checked(Ok(Some(release))));
    assert!(matches!(updater.state(), State::Manual(_, why) if why == "a development build"));

    updater.kind = appimage();
    let release = parse_latest(answer("v0.3.0").as_bytes(), "0.2.0", &updater.kind)
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("0.3.0 is newer"));
    updater.apply(Event::Checked(Ok(Some(release))));
    assert!(matches!(updater.state(), State::Available(_)));
}

#[test]
fn the_repository_slug_comes_from_the_manifest() {
    assert_eq!(slug(), "Ch3w3y/chewvy");
}

#[test]
fn a_linux_install_is_an_appimage_only_when_started_from_one() {
    if !cfg!(all(target_os = "linux", target_arch = "x86_64")) {
        return;
    }
    let exe = std::env::current_exe().unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(
        install::detect_from(Some(&exe), Some(exe.clone())),
        Kind::AppImage(exe.clone())
    );
    assert!(matches!(
        install::detect_from(Some(&exe), None),
        Kind::Unmanaged(_)
    ));
    let missing = PathBuf::from("/nonexistent/chewvy.AppImage");
    assert!(matches!(
        install::detect_from(Some(&exe), Some(missing)),
        Kind::Unmanaged(_)
    ));
}

/// The whole Linux path against the real published release: the check, the
/// checksum file, the download and the swap. It downloads the AppImage
/// (about 110 MB), so it runs only when asked:
/// `cargo test -p plex-app --lib update::tests::live -- --ignored`.
#[test]
#[ignore = "downloads the published AppImage from GitHub"]
fn live_an_older_build_downloads_verifies_and_swaps_in_the_published_appimage() {
    let dir = std::env::temp_dir().join(format!("chewvy-update-test-{}", std::process::id()));
    std::fs::create_dir_all(&dir).unwrap_or_else(|e| panic!("{e}"));
    let target = dir.join("chewvy");
    std::fs::write(&target, b"the old version").unwrap_or_else(|e| panic!("{e}"));
    let kind = Kind::AppImage(target.clone());

    let release = latest(&kind, "0.0.1")
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("a published release is newer than 0.0.1"));
    let package = release
        .package
        .unwrap_or_else(|| panic!("an AppImage is attached"));
    let sums = release
        .sums
        .unwrap_or_else(|| panic!("SHA256SUMS is attached"));
    let seen = std::cell::Cell::new(0_u64);
    let last = std::cell::Cell::new("");
    install::run(
        &kind,
        &package,
        &sums,
        &|done| seen.set(done),
        &|step, _| {
            last.set(step);
        },
    )
    .unwrap_or_else(|e| panic!("{e}"));

    assert_eq!(last.get(), "Finished");
    assert_eq!(seen.get(), package.size);
    let meta = std::fs::metadata(&target).unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(meta.len(), package.size);
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt as _;
        assert_eq!(meta.permissions().mode() & 0o777, 0o755);
    }
    let mut head = [0_u8; 4];
    std::io::Read::read_exact(
        &mut std::fs::File::open(&target).unwrap_or_else(|e| panic!("{e}")),
        &mut head,
    )
    .unwrap_or_else(|e| panic!("{e}"));
    assert_eq!(&head, b"\x7fELF");
    assert!(!dir.join(".chewvy-update.part").exists());
    let _ = std::fs::remove_dir_all(&dir);
}

#[test]
fn an_install_moves_from_download_through_its_steps_to_a_restart() {
    let mut updater = Updater {
        kind: appimage(),
        ..Updater::default()
    };
    let release = parse_latest(answer("v0.3.0").as_bytes(), "0.2.0", &updater.kind)
        .unwrap_or_else(|e| panic!("{e}"))
        .unwrap_or_else(|| panic!("0.3.0 is newer"));
    updater.apply(Event::Checked(Ok(Some(release.clone()))));
    // What `install` sets before its job starts, without the download.
    updater.state = State::Downloading {
        release: release.clone(),
        done: 0,
        total: 1234,
    };
    assert!(updater.busy());
    updater.apply(Event::Progress(600));
    assert!(matches!(
        updater.state,
        State::Downloading {
            done: 600,
            total: 1234,
            ..
        }
    ));
    updater.apply(Event::Installing("Checking the download", Some(1.0)));
    updater.apply(Event::Installing("Replacing the AppImage", None));
    assert!(matches!(
        updater.state,
        State::Installing {
            step: "Replacing the AppImage",
            fraction: None,
            ..
        }
    ));
    updater.apply(Event::Installed(Ok(())));
    assert_eq!(updater.state, State::Ready(release.clone()));
    assert!(!updater.busy());

    // A failed install keeps the offer, so trying again needs no new check.
    updater.state = State::Installing {
        release: release.clone(),
        step: "Running the installer",
        fraction: None,
    };
    updater.apply(Event::Installed(Err("no room".to_owned())));
    assert_eq!(updater.state, State::Failed("no room".to_owned()));
    assert_eq!(updater.offered(), Some(&release));
}
