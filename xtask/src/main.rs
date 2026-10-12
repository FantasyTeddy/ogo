//! Build tasks for ogo, run on the PC: `cargo xtask <command>` (alias in .cargo/config.toml).
//!
//! - `pack <game>`: builds a game and writes an `.ogo` package, or copies it to an SD card.
//! - `release`: builds the launcher, the recovery app and all games into `dist/`, plus a complete
//!   flash image for installing over USB without building.
//! - `flash`: builds and flashes the launcher (and with `--all` the recovery app) over USB.
//! - `test`: runs the tests on the PC (`ogo-format`).
//! - `samples <sd-root>`: writes fake games to an SD card, to test the launcher's game list.
//! - `icon <game> <image.png>`: converts a PNG into the game's `icon.rgb565`.
//!
//! Every firmware image built here carries the git commit (`OGO_COMMIT`, see
//! `ogo_board::image_desc!`).

mod icon;
mod samples;

use std::{
    env, fs, io,
    path::{Path, PathBuf},
    process::Command,
    time::{SystemTime, UNIX_EPOCH},
};

use anyhow::{bail, ensure, Context};
use clap::{Parser, Subcommand};
use ogo_format::{
    flash,
    game::{Manifest, SDK_VERSION},
    image::{ImageDesc, ImageKind},
    slots,
};
use serde::{de::DeserializeOwned, Deserialize};
use sha2::{Digest, Sha256};

/// First byte of every ESP32 app image.
const IMAGE_MAGIC: u8 = 0xE9;
const DIST_DIR: &str = "dist";
const LAUNCHER: &str = "ogo-launcher";
const RECOVERY: &str = "ogo-recovery";
/// The PC, for host builds; the workspace's default target is the ESP32.
const HOST_TARGET: &str = "host-tuple";

#[derive(Parser)]
#[command(about = "Build tasks for ogo")]
struct Cli {
    #[command(subcommand)]
    command: Task,
}

#[derive(Subcommand)]
enum Task {
    /// Build a game and write an .ogo package (default: dist/<id>.ogo), or copy it to an SD card.
    Pack {
        /// The game's package name, e.g. snake.
        game: String,
        /// Write the package to this file.
        #[arg(long, conflicts_with = "sd")]
        ogo: Option<PathBuf>,
        /// Copy the game to this SD card root (e.g. E:\) as /ogo/games/<id>/.
        #[arg(long)]
        sd: Option<PathBuf>,
    },
    /// Build launcher.bin, recovery.bin and an .ogo for every game into dist/.
    Release {
        /// Cargo features for the launcher, e.g. rollback-test.
        #[arg(long)]
        features: Option<String>,
    },
    /// Build and flash over USB, with this project's bootloader and partition table.
    Flash {
        /// Serial port, e.g. COM7. Without it, espflash picks or asks.
        #[arg(long)]
        port: Option<String>,
        /// Also flash the recovery app; needed the first time and after a partition table change.
        #[arg(long)]
        all: bool,
        /// Open the serial monitor afterwards.
        #[arg(long)]
        monitor: bool,
        /// Cargo features for the launcher, e.g. rollback-test.
        #[arg(long)]
        features: Option<String>,
    },
    /// Run the tests on the PC (ogo-format).
    Test,
    /// Convert a PNG into the game's icon.rgb565 (64x64, transparent parts black).
    Icon {
        /// The game's package name, e.g. snake.
        game: String,
        /// A PNG, ideally square.
        image: PathBuf,
    },
    /// Write fake games (not runnable, two broken on purpose) to an SD card, to test the game list.
    Samples {
        /// The SD card root, e.g. E:\.
        sd: PathBuf,
        /// Overwrite sample folders that already exist.
        #[arg(long)]
        force: bool,
    },
}

fn main() -> anyhow::Result<()> {
    let cli = Cli::parse();
    // Needs neither the toolchain nor git.
    if let Task::Samples { sd, force } = &cli.command {
        return samples::write(sd, *force);
    }
    let project = Project::new()?;
    match cli.command {
        Task::Pack { game, ogo, sd } => {
            let game = project.game(&game)?;
            let files = project.build_game(&game)?;
            match sd {
                Some(sd) => install_to_sd(&files, &sd),
                None => {
                    let path = ogo.unwrap_or_else(|| project.root.join(DIST_DIR).join(format!("{}.ogo", game.id)));
                    write_ogo(&files, &path)
                }
            }
        }
        Task::Release { features } => project.release(features.as_deref()),
        Task::Flash { port, all, monitor, features } => project.flash(port.as_deref(), all, monitor, features.as_deref()),
        Task::Test => project.cargo(&["test", "--manifest-path", "ogo-format/Cargo.toml", "--target", HOST_TARGET]),
        Task::Icon { game, image } => icon::convert(&image, &project.game(&game)?.dir.join("icon.rgb565")),
        Task::Samples { .. } => unreachable!("handled before the project is set up"),
    }
}

struct Project {
    root: PathBuf,
    /// Short commit, e.g. "220357ce"; `None` outside a git checkout.
    commit: Option<String>,
}

/// A game crate: a workspace package with `[package.metadata.ogo]`.
struct Game {
    id: String,
    name: String,
    version: String,
    author: String,
    dir: PathBuf,
}

/// What goes into a game's folder on the SD card, or into its .ogo package.
struct GameFiles {
    id: String,
    files: Vec<(&'static str, Vec<u8>)>,
}

impl Project {
    fn new() -> anyhow::Result<Self> {
        if env::var_os("LIBCLANG_PATH").is_none() {
            bail!(
                "LIBCLANG_PATH isn't set: install the Xtensa toolchain with espup (see the README).                  On Linux and macOS, source the export script espup wrote first"
            );
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).parent().context("xtask has no parent folder")?.to_owned();
        let commit = Command::new("git")
            .args(["rev-parse", "--short=8", "HEAD"])
            .current_dir(&root)
            .output()
            .ok()
            .filter(|output| output.status.success())
            .map(|output| String::from_utf8_lossy(&output.stdout).trim().to_owned());
        match &commit {
            Some(commit) => println!("Commit {commit}"),
            None => println!("Not a git checkout: images won't carry a commit"),
        }
        Ok(Self { root, commit })
    }

    /// `cargo <args>` in the project folder, with the commit for `image_desc!`.
    fn cargo(&self, args: &[&str]) -> anyhow::Result<()> {
        let mut command = Command::new(cargo());
        command.args(args).current_dir(&self.root);
        match &self.commit {
            Some(commit) => command.env("OGO_COMMIT", commit),
            None => command.env_remove("OGO_COMMIT"),
        };
        let status = command.status().context("can't run cargo")?;
        ensure!(status.success(), "cargo {} failed", args.join(" "));
        Ok(())
    }

    /// Builds a package into an app image file and returns its bytes. Checks that the image
    /// describes itself as `kind`, with this commit.
    fn build_image(&self, package: &str, kind: ImageKind, features: Option<&str>, out: &Path) -> anyhow::Result<Vec<u8>> {
        println!("Building {package}...");
        if let Some(parent) = out.parent() {
            fs::create_dir_all(parent)?;
        }
        let out_arg = out.to_string_lossy();
        let mut args = vec!["espflash", "save-image", "--chip", "esp32", "--flash-size", "16mb", "--release", "--package", package];
        if let Some(features) = features {
            args.extend(["--features", features]);
        }
        args.push(&out_arg);
        self.cargo(&args)?;
        let image = fs::read(out).with_context(|| format!("can't read {}", out.display()))?;
        ensure!(image.first() == Some(&IMAGE_MAGIC), "cargo espflash didn't produce an ESP32 app image for {package}");
        let desc = ImageDesc::in_image(&image).with_context(|| format!("{package} has no ogo image description"))?;
        ensure!(
            desc.kind() == Some(kind),
            "{package} is an {} image, expected {}",
            desc.kind_name(),
            kind.name()
        );
        ensure!(desc.commit() == self.commit.as_deref(), "{package} doesn't carry the commit; was it rebuilt?");
        Ok(image)
    }

    fn games(&self) -> anyhow::Result<Vec<Game>> {
        #[derive(Deserialize)]
        struct Metadata {
            packages: Vec<Package>,
        }
        #[derive(Deserialize)]
        struct Package {
            name: String,
            version: String,
            authors: Vec<String>,
            manifest_path: PathBuf,
            metadata: Option<PackageMetadata>,
        }
        #[derive(Deserialize)]
        struct PackageMetadata {
            ogo: Option<OgoMetadata>,
        }
        #[derive(Deserialize)]
        struct OgoMetadata {
            name: Option<String>,
        }

        let metadata: Metadata = self.metadata()?;
        let mut games: Vec<Game> = metadata
            .packages
            .into_iter()
            .filter_map(|package| {
                let ogo = package.metadata?.ogo?;
                Some(Game {
                    name: ogo.name.unwrap_or_else(|| package.name.clone()),
                    author: package.authors.into_iter().next().unwrap_or_default(),
                    dir: package.manifest_path.parent()?.to_owned(),
                    id: package.name,
                    version: package.version,
                })
            })
            .collect();
        games.sort_by(|a, b| a.id.cmp(&b.id));
        Ok(games)
    }

    fn game(&self, id: &str) -> anyhow::Result<Game> {
        let games = self.games()?;
        let names: Vec<&str> = games.iter().map(|game| game.id.as_str()).collect();
        let names = names.join(", ");
        games
            .into_iter()
            .find(|game| game.id == id)
            .with_context(|| format!("no game named {id}; games are: {names}"))
    }

    fn build_game(&self, game: &Game) -> anyhow::Result<GameFiles> {
        ensure!(
            ogo_format::game::is_valid_id(&game.id),
            "package name {:?} is not a valid game id (use a-z, 0-9 and -)",
            game.id
        );
        let target_dir = self.target_dir()?;
        let image = self.build_image(&game.id, ImageKind::Game, None, &target_dir.join("ogo-images").join(format!("{}.bin", game.id)))?;
        let manifest = Manifest {
            id: game.id.clone(),
            name: game.name.clone(),
            version: game.version.clone(),
            author: game.author.clone(),
            // Built against this tree's SDK.
            sdk_version: SDK_VERSION,
            sha256: hex(&Sha256::digest(&image)),
        };
        let mut files = vec![("game.toml", manifest.to_toml().into_bytes()), ("game.bin", image)];
        let icon = game.dir.join("icon.rgb565");
        if icon.is_file() {
            files.push(("icon.rgb565", fs::read(&icon)?));
        }
        Ok(GameFiles { id: game.id.clone(), files })
    }

    fn release(&self, features: Option<&str>) -> anyhow::Result<()> {
        let dist = self.root.join(DIST_DIR);
        let launcher = self.build_image(LAUNCHER, ImageKind::Launcher, features, &dist.join("launcher.bin"))?;
        let recovery = self.build_image(RECOVERY, ImageKind::Recovery, None, &dist.join("recovery.bin"))?;
        let full_name = format!("ogo-{}-full.bin", self.version()?);
        let full = self.full_image(&launcher, &recovery)?;
        fs::write(dist.join(&full_name), &full)?;
        let mut summary = vec![
            (full_name, full.len()),
            ("launcher.bin".to_owned(), launcher.len()),
            ("recovery.bin".to_owned(), recovery.len()),
        ];
        for game in self.games()? {
            let files = self.build_game(&game)?;
            let path = dist.join(format!("{}.ogo", game.id));
            write_ogo(&files, &path)?;
            summary.push((format!("{}.ogo", game.id), fs::metadata(&path)?.len() as usize));
        }
        println!("\nRelease in {} (commit {}):", dist.display(), self.commit.as_deref().unwrap_or("unknown"));
        for (name, size) in summary {
            println!("  {name:<20} {:>6} KB", size / 1024);
        }
        Ok(())
    }

    /// Everything `flash --all` writes, as one image from address 0: bootloader, partition table,
    /// recovery and the launcher in slot A. Between them it's 0xFF, so otadata and NVS end up
    /// erased, as after `flash --all` on a new device.
    fn full_image(&self, launcher: &[u8], recovery: &[u8]) -> anyhow::Result<Vec<u8>> {
        let bootloader = fs::read(self.bootloader()?)?;
        let table_bin = self.target_dir()?.join("ogo-images").join("partition-table.bin");
        self.cargo(&["espflash", "partition-table", "--to-binary", "-o", &table_bin.to_string_lossy(), "partitions.csv"])?;
        let table = fs::read(&table_bin)?;
        let partitions = flash::parse_partitions(&fs::read_to_string(self.root.join("partitions.csv"))?)
            .map_err(|e| anyhow::anyhow!("partitions.csv: {e}"))?;
        let slot = |label: &str, image: &[u8]| -> anyhow::Result<u32> {
            let partition = partitions.iter().find(|p| p.label == label).with_context(|| format!("no partition {label}"))?;
            ensure!(image.len() as u64 <= u64::from(partition.size), "the image doesn't fit into {label}");
            Ok(partition.offset)
        };
        let parts = [
            (flash::BOOTLOADER_OFFSET, bootloader.as_slice()),
            (flash::PARTITION_TABLE_OFFSET, table.as_slice()),
            (slot(slots::RECOVERY, recovery)?, recovery),
            (slot(slots::LAUNCHER_A.label, launcher)?, launcher),
        ];
        flash::merge(&parts).map_err(|e| anyhow::anyhow!("can't merge the flash image: {e}"))
    }

    /// The workspace version of launcher, recovery and SDK.
    fn version(&self) -> anyhow::Result<String> {
        #[derive(Deserialize)]
        struct Metadata {
            packages: Vec<Package>,
        }
        #[derive(Deserialize)]
        struct Package {
            name: String,
            version: String,
        }
        self.metadata::<Metadata>()?
            .packages
            .into_iter()
            .find(|package| package.name == LAUNCHER)
            .map(|package| package.version)
            .context("no ogo-launcher package")
    }

    fn flash(&self, port: Option<&str>, all: bool, monitor: bool, features: Option<&str>) -> anyhow::Result<()> {
        // espflash would use its own default bootloader, which lacks the MENU-for-recovery option.
        self.cargo(&["build", "--release", "--package", LAUNCHER])?;
        let bootloader = self.bootloader()?;
        println!("Bootloader: {}", bootloader.display());
        let bootloader = bootloader.to_string_lossy();

        let mut common = vec!["espflash", "flash", "--release", "--bootloader", &bootloader, "--baud", "921600"];
        if let Some(port) = port {
            common.extend(["--port", port]);
        }
        if all {
            let mut args = common.clone();
            args.extend(["--package", RECOVERY, "--target-app-partition", slots::RECOVERY]);
            self.cargo(&args)?;
        }
        // Clearing otadata makes the freshly flashed launcher_a run even if a web update had
        // selected launcher_b: the bootloader starts recovery, which picks the first valid launcher.
        let mut args = common;
        args.extend(["--package", LAUNCHER, "--target-app-partition", slots::LAUNCHER_A.label, "--erase-parts", "otadata"]);
        if let Some(features) = features {
            args.extend(["--features", features]);
        }
        if monitor {
            args.push("--monitor");
        }
        self.cargo(&args)
    }

    fn target_dir(&self) -> anyhow::Result<PathBuf> {
        #[derive(Deserialize)]
        struct Metadata {
            target_directory: PathBuf,
        }
        Ok(self.metadata::<Metadata>()?.target_directory)
    }

    /// The parts of `cargo metadata` that `T` picks out.
    fn metadata<T: DeserializeOwned>(&self) -> anyhow::Result<T> {
        let output = Command::new(cargo())
            .args(["metadata", "--format-version", "1", "--no-deps"])
            .current_dir(&self.root)
            .output()
            .context("can't run cargo metadata")?;
        ensure!(output.status.success(), "cargo metadata failed: {}", String::from_utf8_lossy(&output.stderr));
        Ok(serde_json::from_slice(&output.stdout)?)
    }

    /// esp-idf-sys builds the bootloader next to ESP-IDF; takes the newest one.
    fn bootloader(&self) -> anyhow::Result<PathBuf> {
        let build = self.target_dir()?.join("xtensa-esp32-espidf").join("release").join("build");
        let mut newest: Option<(SystemTime, PathBuf)> = None;
        for entry in fs::read_dir(&build).with_context(|| format!("can't read {}", build.display()))? {
            let entry = entry?;
            if !entry.file_name().to_string_lossy().starts_with("esp-idf-sys-") {
                continue;
            }
            let path = entry.path().join("out").join("build").join("bootloader").join("bootloader.bin");
            if let Ok(modified) = fs::metadata(&path).and_then(|m| m.modified()) {
                if newest.as_ref().is_none_or(|(time, _)| modified > *time) {
                    newest = Some((modified, path));
                }
            }
        }
        newest.map(|(_, path)| path).with_context(|| format!("no bootloader.bin under {}", build.display()))
    }
}

fn write_ogo(game: &GameFiles, path: &Path) -> anyhow::Result<()> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)?;
    }
    let now = SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_secs());
    let file = fs::File::create(path).with_context(|| format!("can't create {}", path.display()))?;
    let files: Vec<(&str, &[u8])> = game.files.iter().map(|(name, content)| (*name, content.as_slice())).collect();
    ogo_format::package::write(io::BufWriter::new(file), &files, now)?;
    println!("Packed {} into {}", game.id, path.display());
    Ok(())
}

fn install_to_sd(game: &GameFiles, sd_root: &Path) -> anyhow::Result<()> {
    ensure!(sd_root.is_dir(), "{} is not a folder", sd_root.display());
    let target = sd_root.join("ogo").join("games").join(&game.id);
    fs::create_dir_all(&target)?;
    for (name, content) in &game.files {
        fs::write(target.join(name), content)?;
    }
    println!("Installed {} to {}", game.id, target.display());
    Ok(())
}

/// The cargo that runs xtask, so builds use the same toolchain.
fn cargo() -> String {
    env::var("CARGO").unwrap_or_else(|_| "cargo".into())
}

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
