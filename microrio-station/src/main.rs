//! MicroRIO driver station.
//!
//! Multi-node telemetry and control over UDP, joystick mapping, and a log of
//! both directions. Nodes are entered by IP in the profile; there is no
//! discovery protocol and nothing on the board to support one.

// A release build on Windows is a GUI app: without this it opens a console
// window behind the station. Debug builds keep the console, so --version and
// the startup banner can still be read while developing.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

mod clock;
mod config;
mod history;
mod joystick;
mod logger;
mod relay;
mod station;
mod theme;
mod ui;
mod version;

use std::path::PathBuf;
use std::sync::Arc;

use station::Station;
use tokio::sync::mpsc;

/// Hold the single-instance lock for as long as the station runs.
///
/// An advisory lock on a file in the config directory: the kernel drops it
/// when the process ends, however it ends, so a crash cannot leave the next
/// start blocked. `--allow-multiple` skips the check for the case where two
/// stations really are wanted, each driving different boards.
fn single_instance() -> Result<Option<std::fs::File>, String> {
    if std::env::args().any(|a| a == "--allow-multiple") {
        return Ok(None);
    }
    let path = config::default_path().with_file_name("station.lock");
    if let Some(dir) = path.parent() {
        let _ = std::fs::create_dir_all(dir);
    }
    let file = std::fs::OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(&path)
        .map_err(|e| format!("cannot open {}: {e}", path.display()))?;
    match lock_exclusive(&file) {
        Ok(()) => Ok(Some(file)),
        Err(_) => Err("Another MicroRIO station is already running on this computer.".into()),
    }
}

#[cfg(unix)]
fn lock_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::unix::io::AsRawFd;
    // SAFETY: a plain libc call on a file descriptor we own.
    let rc = unsafe { libc::flock(file.as_raw_fd(), libc::LOCK_EX | libc::LOCK_NB) };
    if rc == 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

#[cfg(windows)]
fn lock_exclusive(file: &std::fs::File) -> std::io::Result<()> {
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::Foundation::HANDLE;
    use windows_sys::Win32::Storage::FileSystem::{
        LockFileEx, LOCKFILE_EXCLUSIVE_LOCK, LOCKFILE_FAIL_IMMEDIATELY,
    };
    let mut overlapped = unsafe { std::mem::zeroed() };
    // SAFETY: a plain Win32 call on a handle we own.
    let ok = unsafe {
        LockFileEx(
            file.as_raw_handle() as HANDLE,
            LOCKFILE_EXCLUSIVE_LOCK | LOCKFILE_FAIL_IMMEDIATELY,
            0,
            1,
            0,
            &mut overlapped,
        )
    };
    if ok != 0 {
        Ok(())
    } else {
        Err(std::io::Error::last_os_error())
    }
}

fn main() -> anyhow::Result<()> {
    let args: Vec<String> = std::env::args().collect();

    if args.iter().any(|a| a == "--version" || a == "-V") {
        println!("{}", version::full());
        return Ok(());
    }

    // One station per machine, unless told otherwise. Two stations on one
    // node is genuinely dangerous — they both command it and each sees half
    // its telemetry — and the commonest way to end up with two is to launch
    // the app twice without noticing the first one.
    let _lock = match single_instance() {
        Ok(lock) => lock,
        Err(e) => {
            let msg = format!(
                "{e}\n\nTwo stations talking to one board fight over it: both send \
                 commands and each receives only part of its telemetry.\n\nClose the \
                 other station, or start this one with --allow-multiple if you mean to \
                 run two against different boards."
            );
            eprintln!("{msg}");
            if !args.iter().any(|a| a == "--no-dialog") {
                rfd::MessageDialog::new()
                    .set_title("MicroRIO is already running")
                    .set_description(&msg)
                    .set_level(rfd::MessageLevel::Warning)
                    .show();
            }
            return Ok(());
        }
    };

    // Subcommand: export a log without opening the window.
    if args.len() >= 3 && args[1] == "export" {
        let src = PathBuf::from(&args[2]);
        let dir = args
            .get(3)
            .map(PathBuf::from)
            .unwrap_or_else(|| src.with_extension("csv_export"));
        let files = logger::export_csv(&src, &dir)?;
        for f in &files {
            println!("{}", f.display());
        }
        println!("{} file(s) written to {}", files.len(), dir.display());
        return Ok(());
    }

    // Subcommand: what does gilrs see? Headless, for diagnosing a controller.
    if args.len() >= 2 && args[1] == "joysticks" {
        let secs = args.get(2).and_then(|s| s.parse().ok()).unwrap_or(5);
        return joystick::list_headless(secs);
    }

    // --profile names a file anywhere; with no flag we fall back to the
    // per-user default. The app is expected to serve several machines, so the
    // profile is just a file you point at.
    let config_path = args
        .iter()
        .position(|a| a == "--profile")
        .and_then(|i| args.get(i + 1))
        .map(PathBuf::from)
        .unwrap_or_else(config::default_path);

    let profile = config::load(&config_path)?;
    println!("{}", version::full());
    println!("  profile: {}", config_path.display());
    println!("  nodes:   {}", profile.nodes.len());

    let log_cfg = profile.logging.clone();
    let station = Station::new(profile, Some(config_path.clone()));

    // Logging runs on its own thread so the network path never waits on disk.
    let (log_tx, mut log_rx) = mpsc::unbounded_channel::<logger::Record>();
    *station.log_tx.lock().unwrap() = Some(log_tx.clone());
    if log_cfg.enabled {
        let dir = log_cfg.resolved_dir();
        match logger::Logger::create(&dir) {
            Ok(mut lg) => {
                *station.log_path.lock().unwrap() = Some(lg.path().display().to_string());
                println!("  logging: {}", lg.path().display());
                // Ties the log's elapsed-microsecond stamps to the wall clock,
                // so an export can print the time of day each row was taken.
                let _ = log_tx.send(logger::Record::Meta {
                    started_unix_us: station.started_unix_us,
                });
                std::thread::spawn(move || {
                    let mut since_flush = 0u32;
                    while let Some(rec) = log_rx.blocking_recv() {
                        if lg.write(&rec).is_err() {
                            break;
                        }
                        since_flush += 1;
                        if since_flush >= 200 {
                            let _ = lg.flush();
                            since_flush = 0;
                        }
                    }
                    let _ = lg.flush();
                });
            }
            Err(e) => eprintln!("  logging disabled: {e}"),
        }
    } else {
        std::thread::spawn(move || while log_rx.blocking_recv().is_some() {});
    }

    // One tokio runtime on a background thread; eframe owns the main thread.
    let rt = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()?;

    // One supervisor owns the node links and rebuilds them whenever the
    // profile changes, so opening a different profile takes effect at once.
    {
        let st = station.clone();
        let tx = log_tx.clone();
        rt.spawn(async move { station::supervisor(st, tx).await });
    }
    // Keep the runtime alive for the life of the process.
    std::mem::forget(rt);

    // `--view` opens as a watcher: it never commands anything, and a node it
    // is watching keeps answering to whoever is driving it.
    if args.iter().any(|a| a == "--view") {
        station
            .view_by_default
            .store(true, std::sync::atomic::Ordering::Relaxed);
        println!("  watching only: this station will not drive anything");
    }

    let joy = joystick::spawn(station.clone());

    let start_tab = args
        .iter()
        .position(|a| a == "--tab")
        .and_then(|i| args.get(i + 1))
        .cloned();

    // `--size 1920x1080` opens at a given size. The layout is built around
    // two window shapes, full screen and half a screen, and this is how
    // either one gets reproduced on demand rather than by dragging an edge.
    let size = args
        .iter()
        .position(|a| a == "--size")
        .and_then(|i| args.get(i + 1))
        .and_then(|s| s.split_once(['x', 'X']))
        .and_then(|(w, h)| Some([w.trim().parse::<f32>().ok()?, h.trim().parse::<f32>().ok()?]))
        .unwrap_or([1400.0, 900.0]);

    let app_station: Arc<Station> = station.clone();
    let build = move |renderer| eframe::NativeOptions {
        viewport: egui::ViewportBuilder::default()
            .with_inner_size(size)
            .with_title(format!("MicroRIO Driver Station — {}", version::short())),
        renderer,
        ..Default::default()
    };
    let start = move |cc: &eframe::CreationContext<'_>| {
        let mut app = ui::App::new(app_station.clone(), joy.clone());
        if let Some(t) = &start_tab {
            app.select_tab(t);
        }
        let _ = cc;
        Ok(Box::new(app) as Box<dyn eframe::App>)
    };

    // OpenGL first: it asks least of the machine, and a driver station has to
    // open on whatever laptop is in the room. A box with no usable GL — a
    // remote session, a stripped VM — falls back to wgpu rather than exiting
    // with a graphics error nobody can act on.
    let first = eframe::run_native(
        "MicroRIO Driver Station",
        build(eframe::Renderer::Glow),
        Box::new(start.clone()),
    );
    let Err(e) = first else { return Ok(()) };
    eprintln!("OpenGL did not start ({e}); trying the other renderer.");
    eframe::run_native(
        "MicroRIO Driver Station",
        build(eframe::Renderer::Wgpu),
        Box::new(start),
    )
    .map_err(|e| anyhow::anyhow!("no renderer would start: {e}"))
}
