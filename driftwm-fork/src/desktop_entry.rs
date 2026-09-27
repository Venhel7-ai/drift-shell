//! Resolving a window's `app_id` to a launchable `.desktop` entry, and
//! synthesizing the relaunch command from its `Exec=` line.
//!
//! A suspended window needs a stable identity (so it can be titled, matched,
//! and serialized) and a way to come back (relaunch). Both derive purely from
//! the freedesktop desktop-entry database, scanned once into a cache. Only apps
//! that resolve to an entry are eligible — anything else would leave a
//! suspended window that can never relaunch, permanently inert on the canvas.
//!
//! Resolution never uses substring or glob matching (see [`DesktopEntryCache::resolve`]
//! for the exact match order) — a wrong app must never launch in place of the
//! intended one.
//!
//! Terminal entries (`Terminal=true`) are treated as non-existent: their
//! relaunch would open a terminal, not the app, so a suspended window built
//! from one could never honestly restore itself.

use std::collections::HashSet;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::time::SystemTime;

use freedesktop_desktop_entry::DesktopEntry;

/// A window's resolved identity: the original surface `app_id` (the matching
/// key), the `.desktop` id that launches it, and the human-readable name shown
/// in chrome and the centered label.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppIdentity {
    /// Original surface `app_id` — the key a future window is matched against.
    pub app_id: String,
    /// Resolved `.desktop` id (filename stem) — the launch key.
    pub desktop_id: String,
    /// `Name=` from the entry, unlocalized (falls back to the desktop id).
    pub display_name: String,
}

/// A scanned, deduplicated view of the desktop-entry database.
///
/// Built once (ideally on a background thread — the scan reads and parses every
/// `.desktop` file) and reused for every resolution and relaunch. **Must stay
/// `Send`** so the startup warmer can build it off the event-loop thread; the
/// static assertion below enforces it.
///
/// Staleness is keyed on the scanned directories' mtimes, which change when an
/// entry is added or removed. Editing a file in place does not bump its
/// directory's mtime and so is not detected — an accepted limitation.
#[derive(Debug)]
pub struct DesktopEntryCache {
    dirs: Vec<PathBuf>,
    mtimes: Vec<Option<SystemTime>>,
    entries: Vec<CachedEntry>,
}

const _: fn() = || {
    fn assert_send<T: Send>() {}
    assert_send::<DesktopEntryCache>();
};

#[derive(Debug)]
struct CachedEntry {
    /// Filename stem — both the launch key and the exact-match resolution key.
    stem: String,
    startup_wm_class: Option<String>,
    display_name: String,
    /// Pre-parsed `Exec=` with field codes stripped. Empty when unlaunchable.
    exec: Vec<String>,
    terminal: bool,
}

impl CachedEntry {
    /// An entry is only offered to the canvas if relaunching it actually opens
    /// the app: it must have a runnable command and not be a terminal wrapper.
    fn launchable(&self) -> bool {
        !self.terminal && !self.exec.is_empty()
    }
}

impl DesktopEntryCache {
    /// Scan the given application directories (precedence order: earlier dirs
    /// win, mirroring XDG data-dir precedence). Directories that don't exist or
    /// can't be read are skipped. Only top-level files are scanned — the menu
    /// spec's subdirectory ids (`kde4/foo.desktop` → `kde4-foo`) are out of
    /// scope.
    pub fn new(dirs: Vec<PathBuf>) -> Self {
        let mtimes = dirs.iter().map(|d| dir_mtime(d)).collect();
        let mut entries = Vec::new();
        let mut claimed: HashSet<String> = HashSet::new();

        for dir in &dirs {
            let Ok(read_dir) = std::fs::read_dir(dir) else {
                continue;
            };
            let mut paths: Vec<PathBuf> = read_dir
                .flatten()
                .map(|e| e.path())
                .filter(|p| p.extension().and_then(|e| e.to_str()) == Some("desktop"))
                .collect();
            // read_dir order is arbitrary; sorted, StartupWMClass ties within
            // a directory resolve deterministically.
            paths.sort();
            for path in paths {
                let Some(stem) = path
                    .file_stem()
                    .and_then(|s| s.to_str())
                    .map(str::to_string)
                else {
                    continue;
                };
                // A higher-precedence directory already owns this id — even a
                // hidden or malformed copy shadows lower-precedence ones.
                if !claimed.insert(stem.clone()) {
                    continue;
                }
                let Ok(entry) = DesktopEntry::from_path(&path, None::<&[&str]>) else {
                    continue;
                };
                // `Hidden=true` means "treat as if it does not exist" per the
                // spec — claimed (so it shadows lower copies) but never offered.
                if entry.hidden() {
                    continue;
                }
                let display_name = entry
                    .name(&[] as &[&str])
                    .map(|n| n.into_owned())
                    .unwrap_or_else(|| stem.clone());
                entries.push(CachedEntry {
                    stem,
                    startup_wm_class: entry.startup_wm_class().map(str::to_string),
                    display_name,
                    exec: entry.exec().map(parse_exec_line).unwrap_or_default(),
                    terminal: entry.terminal(),
                });
            }
        }

        Self {
            dirs,
            mtimes,
            entries,
        }
    }

    /// Build from the current environment's XDG application directories.
    pub fn from_env() -> Self {
        Self::new(default_application_dirs())
    }

    /// Resolve a surface `app_id` to an identity, or `None` if no launchable
    /// entry matches. Match order: exact filename stem, then exact
    /// `StartupWMClass`, then case-insensitive filename stem.
    pub fn resolve(&self, app_id: &str) -> Option<AppIdentity> {
        let entry = self
            .entries
            .iter()
            .filter(|e| e.launchable())
            .find(|e| e.stem == app_id)
            .or_else(|| {
                self.entries
                    .iter()
                    .filter(|e| e.launchable())
                    .find(|e| e.startup_wm_class.as_deref() == Some(app_id))
            })
            .or_else(|| {
                self.entries
                    .iter()
                    .filter(|e| e.launchable())
                    .find(|e| e.stem.eq_ignore_ascii_case(app_id))
            })?;

        Some(AppIdentity {
            app_id: app_id.to_string(),
            desktop_id: entry.stem.clone(),
            display_name: entry.display_name.clone(),
        })
    }

    /// The relaunch command (argv, field codes stripped) for a resolved
    /// `desktop_id`. `None` if the entry vanished from the cache since
    /// resolution (e.g. uninstalled). Exec the argv directly or re-quote it —
    /// joining with spaces re-splits arguments that contained whitespace.
    pub fn launch_command(&self, desktop_id: &str) -> Option<Vec<String>> {
        self.entries
            .iter()
            .find(|e| e.launchable() && e.stem == desktop_id)
            .map(|e| e.exec.clone())
    }

    /// True if any scanned directory's mtime differs from the snapshot taken at
    /// build time — an entry was added or removed.
    pub fn is_stale(&self) -> bool {
        self.dirs
            .iter()
            .zip(&self.mtimes)
            .any(|(dir, snapshot)| dir_mtime(dir) != *snapshot)
    }

    /// Rebuild the cache in place if it has gone stale. Returns whether a
    /// rebuild happened.
    pub fn refresh(&mut self) -> bool {
        if self.is_stale() {
            *self = Self::new(self.dirs.clone());
            true
        } else {
            false
        }
    }
}

fn dir_mtime(dir: &Path) -> Option<SystemTime> {
    std::fs::metadata(dir).and_then(|m| m.modified()).ok()
}

/// XDG application directories in precedence order: `$XDG_DATA_HOME/applications`
/// first, then each `$XDG_DATA_DIRS/applications`, with the spec's defaults when
/// unset.
fn default_application_dirs() -> Vec<PathBuf> {
    let mut dirs = Vec::new();

    let data_home = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .filter(|p| p.is_absolute())
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")));
    if let Some(home) = data_home {
        dirs.push(home.join("applications"));
    }

    let data_dirs = std::env::var_os("XDG_DATA_DIRS")
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| OsString::from("/usr/local/share:/usr/share"));
    for base in std::env::split_paths(&data_dirs).filter(|p| p.is_absolute()) {
        dirs.push(base.join("applications"));
    }

    dirs
}

/// Parse an `Exec=` value into an argv, honoring the desktop-entry spec's
/// double-quote quoting and dropping the application field codes
/// (`%f %F %u %U %i %c %k` plus the deprecated ones). `%%` becomes a literal
/// `%`. No URIs or icons are substituted — a relaunch opens the app bare.
fn parse_exec_line(exec: &str) -> Vec<String> {
    // A malformed Exec yields nothing rather than a half-parsed command —
    // the entry becomes unlaunchable, never wrong.
    tokenize(exec)
        .map(|tokens| tokens.into_iter().filter_map(field_code).collect())
        .unwrap_or_default()
}

/// Split on whitespace outside double quotes; inside quotes, a backslash
/// escapes the spec's reserved characters (`"` `` ` `` `$` `\`). `None` on an
/// unmatched quote.
fn tokenize(s: &str) -> Option<Vec<String>> {
    let mut tokens = Vec::new();
    let mut current = String::new();
    let mut has_token = false;
    let mut chars = s.chars().peekable();

    while let Some(c) = chars.next() {
        match c {
            '"' => {
                has_token = true;
                let mut closed = false;
                while let Some(qc) = chars.next() {
                    match qc {
                        '"' => {
                            closed = true;
                            break;
                        }
                        '\\' => match chars.peek() {
                            Some(&next @ ('"' | '`' | '$' | '\\')) => {
                                current.push(next);
                                chars.next();
                            }
                            _ => current.push('\\'),
                        },
                        other => current.push(other),
                    }
                }
                if !closed {
                    return None;
                }
            }
            c if c.is_whitespace() => {
                if has_token {
                    tokens.push(std::mem::take(&mut current));
                    has_token = false;
                }
            }
            other => {
                has_token = true;
                current.push(other);
            }
        }
    }

    if has_token {
        tokens.push(current);
    }
    Some(tokens)
}

/// Drop a token that is exactly a field code; inside larger tokens, codes
/// substitute as empty (glib's lenient handling of spec-violating embeds like
/// `--open=%u`), and `%%` collapses to a literal `%` — so `%%f` stays `%f`.
fn field_code(token: String) -> Option<String> {
    match token.as_str() {
        "%f" | "%F" | "%u" | "%U" | "%i" | "%c" | "%k" | "%d" | "%D" | "%n" | "%N" | "%v"
        | "%m" => None,
        _ => {
            let mut out = String::with_capacity(token.len());
            let mut chars = token.chars().peekable();
            while let Some(c) = chars.next() {
                if c != '%' {
                    out.push(c);
                    continue;
                }
                match chars.peek() {
                    Some('%') => {
                        out.push('%');
                        chars.next();
                    }
                    Some(code) if "fFuUickdDnNvm".contains(*code) => {
                        chars.next();
                    }
                    _ => out.push('%'),
                }
            }
            Some(out)
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    /// Self-cleaning unique temp directory. The bin-crate fixture has its own
    /// (`src/tests/real.rs`), unreachable from a lib-crate unit test, so this
    /// mirrors the pattern here.
    struct TempDir {
        path: PathBuf,
    }

    impl TempDir {
        fn new() -> Self {
            use std::sync::atomic::{AtomicUsize, Ordering};
            static COUNTER: AtomicUsize = AtomicUsize::new(0);
            let n = COUNTER.fetch_add(1, Ordering::Relaxed);
            let path = std::env::temp_dir().join(format!(
                "driftwm-desktop-entry-test-{}-{n}",
                std::process::id()
            ));
            std::fs::create_dir_all(&path).unwrap();
            Self { path }
        }

        fn subdir(&self, name: &str) -> PathBuf {
            let dir = self.path.join(name);
            std::fs::create_dir_all(&dir).unwrap();
            dir
        }
    }

    impl Drop for TempDir {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.path);
        }
    }

    fn write_entry(dir: &Path, filename: &str, contents: &str) {
        let mut file = std::fs::File::create(dir.join(filename)).unwrap();
        file.write_all(contents.as_bytes()).unwrap();
    }

    fn desktop(fields: &str) -> String {
        format!("[Desktop Entry]\nType=Application\n{fields}\n")
    }

    /// foot ships three sibling entries (`foot`, `footclient`, `foot-server`),
    /// all `Terminal=false` (foot *is* a terminal; it is not launched inside
    /// one) and none carrying `StartupWMClass`, so resolution relies on the
    /// app-id matching the filename stem directly.
    #[test]
    fn foot_family_resolves_from_real_world_shapes() {
        let tmp = TempDir::new();
        let home = tmp.subdir("home");
        let system = tmp.subdir("system");
        // System entries (as shipped by the foot package).
        write_entry(
            &system,
            "foot.desktop",
            &desktop("Name=Foot\nExec=foot\nTerminal=false\nCategories=System;TerminalEmulator;"),
        );
        write_entry(
            &system,
            "footclient.desktop",
            &desktop(
                "Name=Foot Client\nExec=footclient\nTerminal=false\nCategories=System;TerminalEmulator;",
            ),
        );
        write_entry(
            &system,
            "foot-server.desktop",
            &desktop(
                "Name=Foot Server\nExec=foot --server\nTerminal=false\nCategories=System;TerminalEmulator;",
            ),
        );
        // A user copy of foot/footclient shadows the system ones (data-home wins).
        write_entry(
            &home,
            "foot.desktop",
            &desktop("Name=Foot\nExec=foot\nTerminal=false"),
        );
        write_entry(
            &home,
            "footclient.desktop",
            &desktop("Name=Foot Client\nExec=footclient\nTerminal=false"),
        );

        let cache = DesktopEntryCache::new(vec![home, system]);

        let foot = cache.resolve("foot").expect("foot resolves");
        assert_eq!(foot.desktop_id, "foot");
        assert_eq!(cache.launch_command("foot").unwrap(), vec!["foot"]);

        let client = cache.resolve("footclient").expect("footclient resolves");
        assert_eq!(client.desktop_id, "footclient");
        assert_eq!(
            cache.launch_command("footclient").unwrap(),
            vec!["footclient"]
        );

        // foot-server lives only in the system dir; its Exec keeps the argument.
        assert_eq!(
            cache.launch_command("foot-server").unwrap(),
            vec!["foot", "--server"]
        );
    }

    #[test]
    fn resolves_by_exact_filename_stem() {
        let tmp = TempDir::new();
        let dir = tmp.subdir("applications");
        write_entry(
            &dir,
            "firefox.desktop",
            &desktop("Name=Firefox\nExec=firefox %u"),
        );

        let cache = DesktopEntryCache::new(vec![dir]);
        let identity = cache.resolve("firefox").unwrap();
        assert_eq!(identity.app_id, "firefox");
        assert_eq!(identity.desktop_id, "firefox");
        assert_eq!(identity.display_name, "Firefox");
    }

    #[test]
    fn resolves_by_startup_wm_class() {
        let tmp = TempDir::new();
        let dir = tmp.subdir("applications");
        write_entry(
            &dir,
            "org.gimp.GIMP.desktop",
            &desktop("Name=GIMP\nStartupWMClass=gimp\nExec=gimp"),
        );

        let cache = DesktopEntryCache::new(vec![dir]);
        let identity = cache.resolve("gimp").unwrap();
        assert_eq!(identity.desktop_id, "org.gimp.GIMP");
        assert_eq!(identity.display_name, "GIMP");
    }

    #[test]
    fn resolves_by_case_insensitive_stem() {
        let tmp = TempDir::new();
        let dir = tmp.subdir("applications");
        write_entry(&dir, "Gimp-2.10.desktop", &desktop("Name=GIMP\nExec=gimp"));

        let cache = DesktopEntryCache::new(vec![dir]);
        assert_eq!(cache.resolve("gimp-2.10").unwrap().desktop_id, "Gimp-2.10");
    }

    #[test]
    fn does_not_match_by_substring() {
        let tmp = TempDir::new();
        let dir = tmp.subdir("applications");
        write_entry(
            &dir,
            "firefox.desktop",
            &desktop("Name=Firefox\nExec=firefox"),
        );

        let cache = DesktopEntryCache::new(vec![dir]);
        assert!(cache.resolve("fire").is_none());
        assert!(cache.resolve("firefox-esr").is_none());
    }

    #[test]
    fn unresolvable_app_id_returns_none() {
        let tmp = TempDir::new();
        let dir = tmp.subdir("applications");
        write_entry(
            &dir,
            "firefox.desktop",
            &desktop("Name=Firefox\nExec=firefox"),
        );

        let cache = DesktopEntryCache::new(vec![dir]);
        assert!(cache.resolve("nonexistent").is_none());
    }

    #[test]
    fn data_home_wins_over_data_dirs() {
        let tmp = TempDir::new();
        let home = tmp.subdir("home");
        let system = tmp.subdir("system");
        write_entry(
            &home,
            "editor.desktop",
            &desktop("Name=Home Editor\nExec=home-editor"),
        );
        write_entry(
            &system,
            "editor.desktop",
            &desktop("Name=System Editor\nExec=system-editor"),
        );

        let cache = DesktopEntryCache::new(vec![home, system]);
        assert_eq!(cache.resolve("editor").unwrap().display_name, "Home Editor");
        assert_eq!(cache.launch_command("editor").unwrap(), vec!["home-editor"]);
    }

    #[test]
    fn earlier_data_dir_wins_over_later() {
        let tmp = TempDir::new();
        let first = tmp.subdir("first");
        let second = tmp.subdir("second");
        write_entry(&first, "editor.desktop", &desktop("Name=First\nExec=first"));
        write_entry(
            &second,
            "editor.desktop",
            &desktop("Name=Second\nExec=second"),
        );

        let cache = DesktopEntryCache::new(vec![first, second]);
        assert_eq!(cache.resolve("editor").unwrap().display_name, "First");
    }

    #[test]
    fn terminal_entries_are_not_resolvable() {
        let tmp = TempDir::new();
        let dir = tmp.subdir("applications");
        write_entry(
            &dir,
            "htop.desktop",
            &desktop("Name=htop\nExec=htop\nTerminal=true"),
        );

        let cache = DesktopEntryCache::new(vec![dir]);
        assert!(cache.resolve("htop").is_none());
        assert!(cache.launch_command("htop").is_none());
    }

    #[test]
    fn hidden_entries_are_not_resolvable() {
        let tmp = TempDir::new();
        let dir = tmp.subdir("applications");
        write_entry(
            &dir,
            "ghost.desktop",
            &desktop("Name=Ghost\nExec=ghost\nHidden=true"),
        );

        let cache = DesktopEntryCache::new(vec![dir]);
        assert!(cache.resolve("ghost").is_none());
    }

    #[test]
    fn hidden_high_precedence_shadows_lower_copy() {
        let tmp = TempDir::new();
        let home = tmp.subdir("home");
        let system = tmp.subdir("system");
        write_entry(
            &home,
            "app.desktop",
            &desktop("Name=Hidden\nExec=app\nHidden=true"),
        );
        write_entry(&system, "app.desktop", &desktop("Name=Visible\nExec=app"));

        let cache = DesktopEntryCache::new(vec![home, system]);
        // The higher-precedence Hidden entry removes the id entirely.
        assert!(cache.resolve("app").is_none());
    }

    #[test]
    fn no_display_entries_are_resolvable() {
        let tmp = TempDir::new();
        let dir = tmp.subdir("applications");
        write_entry(
            &dir,
            "helper.desktop",
            &desktop("Name=Helper\nExec=helper\nNoDisplay=true"),
        );

        let cache = DesktopEntryCache::new(vec![dir]);
        // NoDisplay hides from menus but stays launchable.
        assert_eq!(cache.resolve("helper").unwrap().display_name, "Helper");
    }

    #[test]
    fn entry_without_exec_is_not_resolvable() {
        let tmp = TempDir::new();
        let dir = tmp.subdir("applications");
        write_entry(&dir, "link.desktop", &desktop("Name=Link"));

        let cache = DesktopEntryCache::new(vec![dir]);
        assert!(cache.resolve("link").is_none());
    }

    #[test]
    fn display_name_falls_back_to_desktop_id() {
        let tmp = TempDir::new();
        let dir = tmp.subdir("applications");
        write_entry(&dir, "nameless.desktop", &desktop("Exec=nameless"));

        let cache = DesktopEntryCache::new(vec![dir]);
        assert_eq!(cache.resolve("nameless").unwrap().display_name, "nameless");
    }

    #[test]
    fn launch_command_returns_stripped_argv() {
        let tmp = TempDir::new();
        let dir = tmp.subdir("applications");
        write_entry(
            &dir,
            "viewer.desktop",
            &desktop("Name=Viewer\nExec=viewer --new %F"),
        );

        let cache = DesktopEntryCache::new(vec![dir]);
        assert_eq!(
            cache.launch_command("viewer").unwrap(),
            vec!["viewer", "--new"]
        );
    }

    #[test]
    fn exec_strips_application_field_codes() {
        assert_eq!(parse_exec_line("firefox %u"), vec!["firefox"]);
        assert_eq!(parse_exec_line("app %f %F %u %U %i %c %k"), vec!["app"]);
        assert_eq!(
            parse_exec_line("app --flag %F --other"),
            vec!["app", "--flag", "--other"]
        );
    }

    #[test]
    fn exec_converts_double_percent_to_literal() {
        assert_eq!(parse_exec_line("prog 100%%done"), vec!["prog", "100%done"]);
        assert_eq!(parse_exec_line("prog %%"), vec!["prog", "%"]);
        assert_eq!(parse_exec_line("prog %%f"), vec!["prog", "%f"]);
    }

    #[test]
    fn exec_substitutes_embedded_field_codes_as_empty() {
        assert_eq!(parse_exec_line("app --open=%u"), vec!["app", "--open="]);
        assert_eq!(parse_exec_line("app pre%Fpost"), vec!["app", "prepost"]);
        assert_eq!(parse_exec_line("prog 100%%f%u"), vec!["prog", "100%f"]);
        assert_eq!(parse_exec_line("prog 50%off"), vec!["prog", "50%off"]);
    }

    #[test]
    fn exec_honors_double_quoted_arguments() {
        assert_eq!(
            parse_exec_line(r#"prog "hello world" tail"#),
            vec!["prog", "hello world", "tail"]
        );
        assert_eq!(
            parse_exec_line(r#"prog "a \"quoted\" b""#),
            vec!["prog", r#"a "quoted" b"#]
        );
    }

    #[test]
    fn exec_empty_yields_no_args() {
        assert!(parse_exec_line("").is_empty());
        assert!(parse_exec_line("   ").is_empty());
    }

    #[test]
    fn cache_is_fresh_after_build() {
        let tmp = TempDir::new();
        let dir = tmp.subdir("applications");
        write_entry(&dir, "a.desktop", &desktop("Name=A\nExec=a"));

        let cache = DesktopEntryCache::new(vec![dir]);
        assert!(!cache.is_stale());
    }

    #[test]
    fn cache_refreshes_when_directory_changes() {
        let tmp = TempDir::new();
        let dir = tmp.subdir("applications");
        write_entry(&dir, "a.desktop", &desktop("Name=A\nExec=a"));

        let mut cache = DesktopEntryCache::new(vec![dir.clone()]);
        assert!(cache.resolve("a").is_some());
        assert!(cache.resolve("b").is_none());
        assert!(!cache.is_stale());

        // Adding an entry bumps the directory mtime — but filesystem
        // timestamp granularity can be coarser than this test's runtime, so
        // force a distinct mtime instead of trusting the write to produce one.
        write_entry(&dir, "b.desktop", &desktop("Name=B\nExec=b"));
        let past = SystemTime::UNIX_EPOCH + std::time::Duration::from_secs(1);
        std::fs::File::open(&dir)
            .unwrap()
            .set_modified(past)
            .unwrap();
        assert!(cache.is_stale());

        assert!(cache.refresh());
        assert!(cache.resolve("b").is_some());
        assert!(!cache.is_stale());
        assert!(!cache.refresh());
    }

    #[test]
    fn cache_goes_stale_when_missing_directory_appears() {
        let tmp = TempDir::new();
        let dir = tmp.path.join("applications");

        let mut cache = DesktopEntryCache::new(vec![dir.clone()]);
        assert!(!cache.is_stale());

        std::fs::create_dir_all(&dir).unwrap();
        write_entry(&dir, "a.desktop", &desktop("Name=A\nExec=a"));
        assert!(cache.is_stale());

        assert!(cache.refresh());
        assert!(cache.resolve("a").is_some());
    }

    #[test]
    fn unmatched_exec_quote_makes_entry_unlaunchable() {
        let tmp = TempDir::new();
        let dir = tmp.subdir("applications");
        write_entry(
            &dir,
            "broken.desktop",
            &desktop("Name=Broken\nExec=broken \"unterminated"),
        );

        let cache = DesktopEntryCache::new(vec![dir]);
        assert!(cache.resolve("broken").is_none());
        assert!(cache.launch_command("broken").is_none());
    }
}
