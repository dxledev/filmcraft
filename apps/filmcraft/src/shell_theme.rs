//! The desktop shell's theme palette, for Settings ▸ Appearance ▸ Shell Theme.
//!
//! Linux shells (Caelestia, Noctalia, Island, Waybar setups) write the theme they apply under
//! `$XDG_CONFIG_HOME/themes`: `.caelestia-use` is the link every application follows, whether the
//! palette is generated from the wallpaper (`.dynamic/<shell>`) or a static theme (`current`).
//! Palettes are `Colors.qml` files, with the shell's JSON palette as a fallback.
//!
//! The files are read once at start-up, so the first frame is already tinted, and then a worker
//! thread looks again every [`POLL`]. Theme switches retarget symlinks and static files keep old
//! timestamps, so the thread compares what it parses, not modification times. It wakes the UI
//! only when a palette changes, and a missing or broken file keeps the last good palette.

// The palette readers only run on Linux; elsewhere the service is `None`.
#![cfg_attr(not(target_os = "linux"), allow(dead_code))]

use std::path::{Path, PathBuf};

use filmcraft_ui_egui::shell_theme::{MAX_PALETTE_BYTES, ShellPalette, ShellSource, parse_palette};

/// How often the worker looks for a changed palette.
#[cfg(target_os = "linux")]
const POLL: std::time::Duration = std::time::Duration::from_millis(700);

/// The palette files of a theme folder, best first. `Colors.qml` leads: the shell's JSON copies of
/// a theme can lag behind it.
const FILES: [&str; 3] = ["Colors.qml", "palette.json", "noctalia.json"];

fn absolute_env(name: &str) -> Option<PathBuf> {
    std::env::var_os(name).map(PathBuf::from).filter(|p| p.is_absolute())
}

fn home_relative(var: &str, fallback: &[&str]) -> Option<PathBuf> {
    absolute_env(var).or_else(|| absolute_env("HOME").map(|h| fallback.iter().fold(h, |p, s| p.join(s))))
}

/// The palette files to try for `source`, in order of preference.
pub fn candidates(source: ShellSource, config: &Path, state: &Path) -> Vec<PathBuf> {
    let themes = config.join("themes");
    let in_dir = |dir: &Path| FILES.iter().map(|f| dir.join(f)).collect::<Vec<_>>();
    let caelestia_state = state.join("caelestia").join("shell-theme-palette.json");
    let mut out = Vec::new();
    match source {
        ShellSource::Auto => {
            out.extend(in_dir(&themes.join(".caelestia-use")));
            out.extend(in_dir(&themes.join("current")));
            out.push(caelestia_state);
        }
        ShellSource::Static => out.extend(in_dir(&themes.join("current"))),
        ShellSource::Dynamic => {
            let dynamic = themes.join(".dynamic");
            // The dynamic palette the shell applies right now, when it applies one.
            if let (Ok(selected), Ok(root)) = (std::fs::canonicalize(themes.join(".caelestia-use")), std::fs::canonicalize(&dynamic))
                && selected.starts_with(&root)
            {
                out.extend(in_dir(&selected));
            }
            out.push(caelestia_state);
            // Otherwise the most recently generated one.
            let mut generated: Vec<(std::time::SystemTime, PathBuf)> = std::fs::read_dir(&dynamic)
                .into_iter()
                .flatten()
                .flatten()
                .filter(|e| e.path().is_dir())
                .filter_map(|e| Some((e.path().join(FILES[0]).metadata().ok()?.modified().ok()?, e.path())))
                .collect();
            generated.sort_by_key(|(modified, _)| std::cmp::Reverse(*modified));
            out.extend(generated.iter().flat_map(|(_, dir)| in_dir(dir)));
        }
    }
    out
}

/// The first of `files` that is a readable, valid palette.
fn first_palette(files: &[PathBuf]) -> Option<ShellPalette> {
    files.iter().find_map(|path| {
        let meta = std::fs::metadata(path).ok()?;
        if !meta.is_file() || meta.len() > MAX_PALETTE_BYTES as u64 {
            return None;
        }
        parse_palette(&std::fs::read_to_string(path).ok()?)
    })
}

/// The current palette for `source` under the given config and state directories.
pub fn read(source: ShellSource, config: &Path, state: &Path) -> Option<ShellPalette> {
    first_palette(&candidates(source, config, state))
}

const SOURCES: [ShellSource; 3] = [ShellSource::Auto, ShellSource::Dynamic, ShellSource::Static];

fn slot(source: ShellSource) -> usize {
    SOURCES.iter().position(|s| *s == source).unwrap_or(0)
}

type Palettes = [Option<ShellPalette>; 3];

/// Re-read every source; keep the last good palette of one that cannot be read now. Returns
/// whether anything changed.
fn refresh(palettes: &mut Palettes, config: &Path, state: &Path) -> bool {
    let mut changed = false;
    for source in SOURCES {
        if let Some(found) = read(source, config, state)
            && palettes[slot(source)] != Some(found)
        {
            palettes[slot(source)] = Some(found);
            changed = true;
        }
    }
    changed
}

/// The host hook (`HostHooks::shell_palette`), or `None` where there is no shell theme to follow.
pub fn service() -> Option<filmcraft_ui_egui::ShellPaletteFn> {
    #[cfg(target_os = "linux")]
    {
        use std::sync::{Arc, Mutex, OnceLock, PoisonError};
        let config = home_relative("XDG_CONFIG_HOME", &[".config"])?;
        let state = home_relative("XDG_STATE_HOME", &[".local", "state"])?;
        let mut first = Palettes::default();
        refresh(&mut first, &config, &state);
        let shared = Arc::new(Mutex::new(first));
        let wake: Arc<OnceLock<egui::Context>> = Arc::new(OnceLock::new());
        let (worker_shared, worker_wake) = (Arc::clone(&shared), Arc::clone(&wake));
        let spawned = std::thread::Builder::new().name("shell-theme".into()).spawn(move || {
            loop {
                std::thread::sleep(POLL);
                let tick = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
                    let mut next = *worker_shared.lock().unwrap_or_else(PoisonError::into_inner);
                    if refresh(&mut next, &config, &state) {
                        *worker_shared.lock().unwrap_or_else(PoisonError::into_inner) = next;
                        if let Some(ctx) = worker_wake.get() {
                            ctx.request_repaint();
                        }
                    }
                }));
                if tick.is_err() {
                    log::warn!("the shell theme watcher failed; Shell Theme keeps the last palette");
                }
            }
        });
        if spawned.is_err() {
            return None;
        }
        Some(Box::new(move |ctx: &egui::Context, source: ShellSource| {
            let _ = wake.set(ctx.clone());
            shared.lock().unwrap_or_else(PoisonError::into_inner)[slot(source)]
        }))
    }
    #[cfg(not(target_os = "linux"))]
    {
        None
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const ISLAND: &str = "property color background: \"#141520\"\nproperty color foreground: \"#e2c9b4\"\nproperty color primary: \"#9badd9\"\n";
    const FIRESKY: &str = "property color background: \"#0f0f0f\"\nproperty color foreground: \"#e0d8d0\"\nproperty color primary: \"#e66a4d\"\n";

    struct Home(PathBuf);
    impl Home {
        fn new(name: &str) -> Home {
            let dir = std::env::temp_dir().join(format!("filmcraft-shell-theme-{name}-{}", std::process::id()));
            let _ = std::fs::remove_dir_all(&dir);
            std::fs::create_dir_all(dir.join("config/themes/.dynamic/island")).unwrap();
            std::fs::create_dir_all(dir.join("config/themes/firesky")).unwrap();
            std::fs::create_dir_all(dir.join("state")).unwrap();
            Home(dir)
        }
        fn config(&self) -> PathBuf {
            self.0.join("config")
        }
        fn state(&self) -> PathBuf {
            self.0.join("state")
        }
        fn write(&self, rel: &str, text: &str) {
            let path = self.0.join(rel);
            std::fs::create_dir_all(path.parent().unwrap()).unwrap();
            std::fs::write(path, text).unwrap();
        }
        #[cfg(unix)]
        fn link(&self, from: &str, to: &str) {
            let from = self.0.join(from);
            let _ = std::fs::remove_file(&from);
            std::os::unix::fs::symlink(self.0.join(to), from).unwrap();
        }
    }
    impl Drop for Home {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.0);
        }
    }

    #[cfg(unix)]
    #[test]
    fn auto_follows_the_application_selector_through_a_theme_switch() {
        let home = Home::new("switch");
        home.write("config/themes/.dynamic/island/Colors.qml", ISLAND);
        home.write("config/themes/firesky/Colors.qml", FIRESKY);
        home.link("config/themes/.caelestia-use", "config/themes/.dynamic/island");
        let island = read(ShellSource::Auto, &home.config(), &home.state()).unwrap();
        assert_eq!(island.primary, [0x9b, 0xad, 0xd9]);
        // the shell retargets the link to a static theme: same files, new palette
        home.link("config/themes/.caelestia-use", "config/themes/firesky");
        let firesky = read(ShellSource::Auto, &home.config(), &home.state()).unwrap();
        assert_eq!(firesky.primary, [0xe6, 0x6a, 0x4d]);
        // and `refresh` reports exactly that change, once
        let mut palettes = Palettes::default();
        home.link("config/themes/.caelestia-use", "config/themes/.dynamic/island");
        assert!(refresh(&mut palettes, &home.config(), &home.state()));
        assert!(!refresh(&mut palettes, &home.config(), &home.state()));
        home.link("config/themes/.caelestia-use", "config/themes/firesky");
        assert!(refresh(&mut palettes, &home.config(), &home.state()));
        assert_eq!(palettes[slot(ShellSource::Auto)], Some(firesky));
    }

    #[cfg(unix)]
    #[test]
    fn static_reads_current_and_dynamic_reads_generated_palettes() {
        let home = Home::new("sources");
        home.write("config/themes/.dynamic/island/Colors.qml", ISLAND);
        home.write("config/themes/firesky/Colors.qml", FIRESKY);
        home.link("config/themes/current", "config/themes/firesky");
        home.link("config/themes/.caelestia-use", "config/themes/firesky");
        assert_eq!(read(ShellSource::Static, &home.config(), &home.state()).unwrap().primary, [0xe6, 0x6a, 0x4d]);
        // the selector is on a static theme: dynamic still finds the generated palette
        assert_eq!(read(ShellSource::Dynamic, &home.config(), &home.state()).unwrap().primary, [0x9b, 0xad, 0xd9]);
        // the shell's own palette file wins over folders it did not select
        home.write("state/caelestia/shell-theme-palette.json", r#"{"colours":{"background":"0a191c","foreground":"ddcbb1","accent":"8eb1dd"}}"#);
        assert_eq!(read(ShellSource::Dynamic, &home.config(), &home.state()).unwrap().primary, [0x8e, 0xb1, 0xdd]);
    }

    #[test]
    fn missing_broken_and_huge_palettes_give_nothing() {
        let home = Home::new("hostile");
        assert_eq!(read(ShellSource::Auto, &home.config(), &home.state()), None);
        home.write("config/themes/.dynamic/island/Colors.qml", "not a palette");
        home.write("config/themes/current/Colors.qml", &"x".repeat(MAX_PALETTE_BYTES + 10));
        assert_eq!(read(ShellSource::Dynamic, &home.config(), &home.state()), None);
        assert_eq!(read(ShellSource::Static, &home.config(), &home.state()), None);
        // a broken file keeps the last good palette in `refresh`
        let mut palettes = Palettes::default();
        home.write("config/themes/current/Colors.qml", FIRESKY);
        assert!(refresh(&mut palettes, &home.config(), &home.state()));
        home.write("config/themes/current/Colors.qml", "{{{");
        assert!(!refresh(&mut palettes, &home.config(), &home.state()));
        assert!(palettes[slot(ShellSource::Static)].is_some());
    }
}
