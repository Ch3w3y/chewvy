//! `chewvy-cli`: the display-free entry point.
//!
//! `status` prints a machine-readable readiness report built from the
//! build-script native probes. `--smoke-no-display` runs a display-free player smoke session (scripted mock
//! control session + the real libmpv probe run) printed as machine-readable
//! JSON, with an optional `--probe-json <path>` capture of the probe section.
//! It creates no window, contacts no display server or audio device, and touches
//! no Plex account, token, server or library metadata.

#![deny(unsafe_code)]
#![warn(missing_docs)]

use clap::{Args, CommandFactory, Parser, Subcommand};
use serde_json::{json, Map, Value};
use std::process::ExitCode;

#[derive(Parser, Debug)]
#[command(
    name = "chewvy-cli",
    version,
    about = "Chewvy helper: native readiness report and display-free player smoke session.",
    propagate_version = true
)]
struct Cli {
    #[command(subcommand)]
    command: Option<Command>,
    /// Run the display-free player smoke session: scripted mock control
    /// session + real libmpv probe with vo=null/ao=null, printed as JSON.
    #[arg(long, global = true)]
    smoke_no_display: bool,
    /// Fixture media paths for the probe (repeatable). Generated locally; never
    /// Plex media. `auto` synthesizes `target/probe-fixtures/m4-tone-1s.wav`
    /// (1.000 s 8 kHz mono PCM sine, `plex_player::probe::synthesize_wav`, no
    /// ffmpeg) so `--probe-json` output is reproducible from the repository.
    #[arg(long, global = true)]
    fixture: Vec<String>,
    /// Write only the real-libmpv probe section to this path (machine-readable
    /// probe log). Omit to print without capturing a file.
    #[arg(long, global = true)]
    probe_json: Option<String>,
}

#[derive(Subcommand, Debug)]
enum Command {
    /// Print a machine-readable workspace/native readiness report (JSON).
    Status(StatusArgs),
}

#[derive(Args, Debug)]
struct StatusArgs {
    /// Exit 3 if any native library required by the researched stack is missing.
    #[arg(long)]
    require_native: bool,
}

/// Native versions discovered by `build.rs` at compile time (probe failures are
/// reported as `missing:<lib>` instead of aborting the build).
fn native_versions() -> [(&'static str, &'static str); 5] {
    [
        ("mpv", env!("PLEX_NATIVE_MPV")),
        ("egl", env!("PLEX_NATIVE_EGL")),
        ("gl", env!("PLEX_NATIVE_GL")),
        ("wayland_client", env!("PLEX_NATIVE_WAYLAND_CLIENT")),
        ("xkbcommon", env!("PLEX_NATIVE_XKBCOMMON")),
    ]
}

fn missing_names(native: &[(&str, &str)]) -> Vec<String> {
    native
        .iter()
        .filter(|(_, version)| version.starts_with("missing:"))
        .map(|(name, _)| (*name).to_owned())
        .collect()
}

fn native_object(native: &[(&str, &str)]) -> Map<String, Value> {
    native
        .iter()
        .map(|(name, version)| ((*name).to_owned(), Value::String((*version).to_owned())))
        .collect()
}

fn main() -> ExitCode {
    let cli = Cli::parse();
    if cli.smoke_no_display {
        return smoke_no_display(&cli);
    }
    match cli.command {
        Some(Command::Status(args)) => {
            let native = native_versions();
            let missing = missing_names(&native);
            let report = json!({
                "tool": "chewvy-cli",
                "version": env!("CARGO_PKG_VERSION"),
                "skeleton": true,
                "product_logic": false,
                "requires_display": false,
                "links_layers": ["plex-core", "plex-player"],
                "native": native_object(&native),
                "native_missing": missing,
            });
            println!("{report}");
            if args.require_native && !missing.is_empty() {
                return ExitCode::from(3);
            }
            ExitCode::SUCCESS
        }
        None => {
            // A usage error, and clap's own parser metadata names the available
            // subcommand(s) so the message cannot drift from the real CLI.
            let mut cmd = Cli::command();
            eprintln!("no command or --smoke-no-display given");
            eprintln!("{}", cmd.render_usage());
            for sub in cmd.get_subcommands() {
                let about = sub.get_about().map(|s| s.to_string()).unwrap_or_default();
                eprintln!("  <{}> {}", sub.get_name(), about);
            }
            eprintln!("try --help");
            ExitCode::from(2)
        }
    }
}

/// Resolve + synthesize the `--fixture auto` media under the current workspace.
///
/// The path must be absolute: `ItemSource::parse` accepts absolute paths, and a
/// bare relative token is an unsupported source by design. It is derived from
/// the workspace at run time, so the captured report is regenerable on any host
/// that runs the documented command.
fn fixture_path() -> std::io::Result<std::path::PathBuf> {
    let root = std::env::current_dir()?;
    plex_player::probe::synthesize_wav(&root.join(plex_player::probe::GENERATED_FIXTURE), 1)
}

/// M4 `--smoke-no-display`: mock control session + real display-free libmpv probe.
fn smoke_no_display(cli: &Cli) -> ExitCode {
    // `--fixture auto` synthesizes the probe's own media fixture under gitignored
    // `target/`, so a captured `--probe-json` report can be regenerated from
    // the repository alone (no machine-local paths, no external media).
    let generated = match fixture_path() {
        Ok(path) => Some(path.display().to_string()),
        Err(err) => {
            eprintln!(
                "fixture synthesis failed path={} error={err}",
                plex_player::probe::GENERATED_FIXTURE
            );
            return ExitCode::from(4);
        }
    };
    let owned: Vec<String> = cli
        .fixture
        .iter()
        .map(|f| {
            if f == "auto" {
                generated.clone().unwrap_or_default()
            } else {
                f.clone()
            }
        })
        .collect();
    let fixtures_given: Vec<&str> = if cli.fixture.is_empty() {
        Vec::new()
    } else {
        owned.iter().map(String::as_str).collect()
    };
    let report = plex_player::smoke::run_smoke(&fixtures_given);
    let mock = &report.mock;
    let probe = &report.probe;
    let items: Vec<Value> = probe
        .items
        .iter()
        .map(|item| {
            json!({
                "source": item.source,
                "classification": item.classification,
                "control_phase": item.phase,
                "control_gate": item.gate,
                "deferred": item.deferred,
                "control_flushes": item.flushes,
                "outcome": item.outcome,
                // Both are needed for a saved report to be traceable: `phase`
                // says which control order the item run used (it is reported as
                // `control_phase`), `end_reason` says how the item ended
                // (`outcome=ended` alone does not distinguish eof from stop).
                "end_reason": item.end_reason,
                "issued": item.issued,
                "events": item.events,
                "pumps": item.pumps,
                "errors": item.errors,
                "unmatched": item.unmatched,
                "property_facts": item.property_facts,
                "final_state": item.final_state,
                "drained_total": item.drained_total,
                "wakeups_total": item.wakeups_total,
                "pump_calls": item.pump_calls,
            })
        })
        .collect();
    let probe_json = json!({
        "build": probe.build,
        "runtime": probe.runtime,
        "names": probe.names,
        "items": items,
        "notes": probe.notes,
    });
    let smoke = json!({
        "tool": "chewvy-cli",
        "mode": "smoke-no-display",
        "version": env!("CARGO_PKG_VERSION"),
        "requires_display": false,
        "fixtures_given": fixtures_given,
        "fixture_generator": format!(
            "plex_player::probe::synthesize_wav (1.000 s 8 kHz mono 16-bit PCM sine, written by hand); --fixture auto synthesizes <workspace>/{fixture_const} from the current working directory at run time, so the item paths in this report are run-local but reproducible by that command",
            fixture_const = plex_player::probe::GENERATED_FIXTURE,
        ),
        "libmpv_observed": report.libmpv_observed(),
        "native": native_object(&native_versions()),
        "native_missing": missing_names(&native_versions()),
        "scripted_mock": {
            "note": "wrapper facts only: these are the calls this layer would have sent; libmpv was not consulted",
            "source": mock.source,
            "outcome": mock.outcome,
            "observations": mock.observations,
            "calls": mock.calls,
            "issued": mock.issued,
            "pumps": mock.pumps,
            "events": mock.events,
            "errors": mock.errors,
            "unmatched": mock.unmatched,
            "observed_counts": mock.observed_counts,
            "state": mock.state,
        },
        "libmpv_probe": probe_json,
        "lines": report.lines(),
    });
    println!("{smoke}");
    if let Some(path) = &cli.probe_json {
        let text = serde_json::to_string_pretty(&probe_json).unwrap_or_else(|_| "{}".to_owned());
        if let Err(err) = std::fs::write(path, format!("{text}\n")) {
            eprintln!("probe capture failed path={path} error={err}");
            return ExitCode::from(5);
        }
        eprintln!("probe captured path={path}");
    }
    if report.libmpv_observed() {
        ExitCode::SUCCESS
    } else {
        eprintln!("libmpv was not observed by this run (see libmpv_probe items)");
        ExitCode::from(4)
    }
}
