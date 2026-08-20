// SPDX-FileCopyrightText: 2026 Roman Valls Guimera <brainstorm@nopcode.org>
//
// SPDX-License-Identifier: GPL-3.0-or-later

//! Build automation for ssh-stamp: `cargo xtask <command>`.
//!
//! Replaces the per-board `cargo` aliases that were multiplying with every
//! new board and MCU (see issue #116). All target knowledge lives in
//! `xtask/targets.toml` plus the `[build]` section of each board TOML, so
//! adding a board or a whole new vendor is data, not more aliases.
//!
//! Run `cargo xtask` with no arguments for usage.

mod targets;

use std::path::{Path, PathBuf};
use std::process::{Command, ExitCode};

use targets::{Chip, Registry};

const USAGE: &str = "\
cargo xtask — ssh-stamp build automation

USAGE:
    cargo xtask <command> [args]

COMMANDS:
    list                    List known boards, chips and platforms
    build <target> [opts]   Build a board (binary) or a bare chip (library)
    run <target> [opts]     Build, flash and monitor a board
    clippy [<target>]       Lint (defaults to the registry's default board)
    fmt [--check]           Format the workspace
    doc [--open]            Build the rustdoc for all library crates,
                            including each platform's board pin catalog
    test                    Run the host-side test suites
    ci                      Everything CI checks: every target, lints, format

OPTIONS:
    --features <a,b>        Extra cargo features, comma or space separated
    --profile <name>        Cargo profile override (default: release)
    --check                 For `fmt`: check instead of rewriting
    --open                  For `doc`: open the docs in a browser afterwards
    -- <args...>            Everything after `--` is passed through to cargo

EXAMPLES:
    cargo xtask build esp32c6-devkitc
    cargo xtask build esp32c3                       # chip only, library build
    cargo xtask run waveshare-esp32-s3-touch-lcd-43 --features can-no-ack
    cargo xtask build esp32c6-devkitc -- --timings
    cargo xtask doc --open                          # docs, then a browser
";

fn main() -> ExitCode {
    match run() {
        Ok(()) => ExitCode::SUCCESS,
        Err(msg) => {
            eprintln!("\nxtask: {msg}");
            ExitCode::FAILURE
        }
    }
}

fn run() -> Result<(), String> {
    let root = workspace_root()?;
    let registry = Registry::load(&root)?;

    let mut raw = std::env::args().skip(1);
    let Some(command) = raw.next() else {
        print!("{USAGE}");
        return Ok(());
    };
    let args = Args::parse(raw)?;

    match command.as_str() {
        "list" => {
            list(&registry);
            Ok(())
        }
        "build" => build(&root, &registry, &args, false),
        "run" => build(&root, &registry, &args, true),
        "clippy" => clippy(&root, &registry, &args),
        "fmt" => fmt(&root, &registry, &args),
        "doc" => doc(&root, &registry, &args),
        "test" => test(&root, &registry),
        "ci" => ci(&root, &registry),
        "-h" | "--help" | "help" => {
            print!("{USAGE}");
            Ok(())
        }
        other => Err(format!("unknown command `{other}`\n\n{USAGE}")),
    }
}

/// The workspace root: this crate lives in `<root>/xtask`.
fn workspace_root() -> Result<PathBuf, String> {
    let manifest = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
    manifest
        .parent()
        .map(Path::to_path_buf)
        .ok_or_else(|| format!("cannot find workspace root above {}", manifest.display()))
}

#[derive(Debug, Default)]
struct Args {
    /// Positional arguments (typically the target name).
    positional: Vec<String>,
    features: Vec<String>,
    profile: Option<String>,
    check: bool,
    open: bool,
    /// Everything after a literal `--`, forwarded to cargo verbatim.
    passthrough: Vec<String>,
}

impl Args {
    fn parse(raw: impl Iterator<Item = String>) -> Result<Self, String> {
        let mut args = Args::default();
        let mut raw = raw.peekable();

        while let Some(arg) = raw.next() {
            match arg.as_str() {
                "--" => {
                    args.passthrough.extend(raw);
                    break;
                }
                "--check" => args.check = true,
                "--open" => args.open = true,
                "--features" => {
                    let value = raw
                        .next()
                        .ok_or_else(|| "--features needs a value".to_string())?;
                    args.features.extend(
                        value
                            .split([',', ' '])
                            .filter(|f| !f.is_empty())
                            .map(str::to_string),
                    );
                }
                "--profile" => {
                    args.profile = Some(
                        raw.next()
                            .ok_or_else(|| "--profile needs a value".to_string())?,
                    );
                }
                other if other.starts_with('-') => {
                    return Err(format!("unknown option `{other}`\n\n{USAGE}"));
                }
                other => args.positional.push(other.to_string()),
            }
        }
        Ok(args)
    }

    fn target(&self) -> Result<&str, String> {
        match self.positional.len() {
            0 => Err("expected a board or chip name (`cargo xtask list`)".to_string()),
            1 => Ok(&self.positional[0]),
            _ => Err(format!(
                "expected one target, got {:?}",
                self.positional.as_slice()
            )),
        }
    }
}

/// What a target name resolved to: a board builds the firmware binary,
/// a bare chip builds the library only.
struct Unit<'a> {
    chip: &'a Chip,
    /// Cargo features selecting the target, before user additions.
    features: Vec<String>,
    /// `--bin <name>` for a board, `--lib` for a bare chip.
    artifact: Vec<String>,
}

fn resolve<'a>(registry: &'a Registry, name: &str) -> Result<Unit<'a>, String> {
    if let Some(board) = registry.boards.get(name) {
        let chip = registry
            .chips
            .get(&board.chip)
            .ok_or_else(|| format!("board `{name}` refers to unknown chip `{}`", board.chip))?;
        let platform = registry.platform_of(chip)?;

        let mut features = vec![format!("{}{}", platform.board_feature_prefix, board.name)];
        features.extend(board.features.iter().cloned());

        return Ok(Unit {
            chip,
            features,
            artifact: vec!["--bin".to_string(), platform.bin.clone()],
        });
    }

    if let Some(chip) = registry.chips.get(name) {
        // No BSP for this chip yet: the binary would hit the "No board
        // feature selected" guard, so build the library instead.
        return Ok(Unit {
            chip,
            features: vec![name.to_string()],
            artifact: vec!["--lib".to_string()],
        });
    }

    Err(format!(
        "unknown target `{name}`; known boards: {}; known chips: {}",
        join(registry.boards.keys()),
        join(registry.chips.keys())
    ))
}

/// A cargo invocation with the parent's toolchain pinning stripped out.
///
/// `cargo xtask` runs us through cargo, which exports `RUSTUP_TOOLCHAIN`,
/// `RUSTC`, `CARGO` and friends. Left in place they would override a
/// `+toolchain` argument (so Xtensa builds would silently use the host
/// toolchain) and shadow `.cargo/config.toml` rustflags.
fn cargo(toolchain: Option<&str>) -> Command {
    let mut cmd = Command::new("cargo");
    for var in [
        "RUSTUP_TOOLCHAIN",
        "RUSTC",
        "RUSTDOC",
        "CARGO",
        "RUSTFLAGS",
        "CARGO_ENCODED_RUSTFLAGS",
    ] {
        cmd.env_remove(var);
    }
    if let Some(toolchain) = toolchain {
        cmd.arg(format!("+{toolchain}"));
    }
    cmd
}

fn exec(root: &Path, mut cmd: Command) -> Result<(), String> {
    cmd.current_dir(root);

    let rendered = format!(
        "cargo {}",
        cmd.get_args()
            .map(|a| a.to_string_lossy().into_owned())
            .collect::<Vec<_>>()
            .join(" ")
    );
    println!("\x1b[1;36m>\x1b[0m {rendered}");

    let status = cmd
        .status()
        .map_err(|e| format!("failed to spawn `{rendered}`: {e}"))?;
    if status.success() {
        Ok(())
    } else {
        Err(format!("`{rendered}` failed with {status}"))
    }
}

/// Explicit `--profile` wins, then the chip's own, else `release`.
fn profile_for(chip: &Chip, args: &Args) -> String {
    args.profile
        .clone()
        .or_else(|| chip.profile.clone())
        .unwrap_or_else(|| "release".to_string())
}

fn build(root: &Path, registry: &Registry, args: &Args, flash: bool) -> Result<(), String> {
    let name = args.target()?;
    let unit = resolve(registry, name)?;
    let platform = registry.platform_of(unit.chip)?;

    let mut features = unit.features;
    features.extend(args.features.iter().cloned());

    let profile = profile_for(unit.chip, args);

    let mut cmd = cargo(registry.toolchain_for(unit.chip));
    cmd.arg(if flash { "run" } else { "build" })
        .args(["--profile", &profile])
        .args(["--target", &unit.chip.target])
        .args(["-p", &platform.package])
        .args(&unit.artifact)
        .arg("--no-default-features")
        .args(["--features", &features.join(",")]);

    if unit.chip.build_std {
        cmd.arg("-Zbuild-std=core,alloc");
    }
    cmd.args(&args.passthrough);

    exec(root, cmd)
}

fn clippy(root: &Path, registry: &Registry, args: &Args) -> Result<(), String> {
    let name = args
        .positional
        .first()
        .cloned()
        .unwrap_or_else(|| registry.defaults.board.clone());
    let unit = resolve(registry, &name)?;
    let platform = registry.platform_of(unit.chip)?;

    let mut features = unit.features;
    features.extend(args.features.iter().cloned());

    let profile = profile_for(unit.chip, args);

    let mut cmd = cargo(registry.toolchain_for(unit.chip));
    cmd.arg("clippy")
        .args(["--profile", &profile])
        .args(["--target", &unit.chip.target])
        .args(["-p", &platform.package])
        .args(&unit.artifact)
        .arg("--no-default-features")
        .args(["--features", &features.join(",")]);

    if unit.chip.build_std {
        cmd.arg("-Zbuild-std=core,alloc");
    }
    cmd.args(&args.passthrough).args(["--", "-D", "warnings"]);

    exec(root, cmd)
}

fn fmt(root: &Path, registry: &Registry, args: &Args) -> Result<(), String> {
    let mut cmd = cargo(registry.host_toolchain());
    cmd.args(["fmt", "--all"]);
    if args.check {
        cmd.args(["--", "--check"]);
    }
    exec(root, cmd)
}

/// Build the rustdoc for the library crates.
///
/// Each platform's BSP build script regenerates its board pin catalog from
/// `boards/*.toml` as part of this, so the table on that crate's front page
/// is never stale.
///
/// With `--open`, the first package in `doc-packages` is opened in a browser
/// once the build succeeds; rustdoc's sidebar reaches the rest of the
/// workspace from there. Cargo's own `--open` would land on `ota`, the
/// alphabetically first of the `-p` flags.
fn doc(root: &Path, registry: &Registry, args: &Args) -> Result<(), String> {
    let board = registry.defaults.board.clone();
    let unit = resolve(registry, &board)?;
    let platform = registry.platform_of(unit.chip)?;

    let mut cmd = cargo(registry.toolchain_for(unit.chip));
    cmd.arg("doc")
        .args(["--target", &unit.chip.target])
        .args(["--no-deps", "--lib"]);
    for package in &registry.defaults.doc_packages {
        cmd.args(["-p", package]);
    }
    // Features must be qualified one by one: `pkg/a,b` would make `b` a
    // feature of the workspace root package rather than of `pkg`.
    let features: Vec<String> = unit
        .features
        .iter()
        .map(|f| format!("{}/{f}", platform.package))
        .collect();
    cmd.arg("--no-default-features")
        .args(["--features", &features.join(",")]);
    if unit.chip.build_std {
        cmd.arg("-Zbuild-std=core,alloc");
    }
    exec(root, cmd)?;

    if args.open {
        let entry = registry
            .defaults
            .doc_packages
            .first()
            .ok_or("doc-packages is empty, nothing to open")?;
        let page = root
            .join("target")
            .join(&unit.chip.target)
            .join("doc")
            .join(entry.replace('-', "_"))
            .join("index.html");
        open(&page)?;
    }
    Ok(())
}

/// Hand a generated page to the desktop's browser.
fn open(page: &Path) -> Result<(), String> {
    let opener = if cfg!(target_os = "macos") {
        "open"
    } else {
        "xdg-open"
    };
    println!("\x1b[1;36m>\x1b[0m {opener} {}", page.display());
    // The browser outlives us, so spawn instead of waiting for it.
    Command::new(opener)
        .arg(page)
        .spawn()
        .map(drop)
        .map_err(|e| format!("`{opener}` failed on {}: {e}", page.display()))
}

fn test(root: &Path, registry: &Registry) -> Result<(), String> {
    // Host-side crates only; the firmware crates are no_std and cannot run
    // tests on the host.
    let mut cmd = cargo(registry.host_toolchain());
    cmd.args(["test", "-p", "ota"]);
    exec(root, cmd)
}

fn ci(root: &Path, registry: &Registry) -> Result<(), String> {
    for name in registry.boards.keys().chain(registry.chips.keys()) {
        let args = Args {
            positional: vec![name.clone()],
            ..Args::default()
        };
        build(root, registry, &args, false)?;
    }
    clippy(root, registry, &Args::default())?;
    fmt(
        root,
        registry,
        &Args {
            check: true,
            ..Args::default()
        },
    )?;
    doc(root, registry, &Args::default())?;
    test(root, registry)
}

fn list(registry: &Registry) {
    println!("Boards (firmware binary):");
    for (name, board) in &registry.boards {
        let extra = if board.features.is_empty() {
            String::new()
        } else {
            format!("  [+{}]", board.features.join(","))
        };
        println!("    {name:<38} {}{extra}", board.chip);
    }

    println!("\nChips (library-only build):");
    for (name, chip) in &registry.chips {
        let toolchain = chip
            .toolchain
            .as_ref()
            .map_or(String::new(), |t| format!("  (+{t})"));
        println!("    {name:<38} {}{toolchain}", chip.target);
    }

    println!("\nPlatforms:");
    for (name, platform) in &registry.platforms {
        println!("    {name:<38} {}", platform.package);
    }
}

fn join<'a>(items: impl Iterator<Item = &'a String>) -> String {
    items.cloned().collect::<Vec<_>>().join(", ")
}
