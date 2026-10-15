//! Structured Windows sources; the shared Scanner probes every candidate JVM.

use std::io::ErrorKind;
use std::path::{Path, PathBuf};

use winreg::enums::{
    HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_READ, KEY_WOW64_32KEY, KEY_WOW64_64KEY,
    REG_EXPAND_SZ, REG_SZ,
};
use winreg::types::FromRegValue;
use winreg::{RegKey, RegValue};

use super::{Scanner, HOME_EXECUTABLES};
use crate::java::JavaDiscovery;

/// Registry value names that hold an installation root. `JavaHome` is the
/// JavaSoft/OpenJDK convention, `InstallationPath` is Zulu's, and `Path` is
/// where the MSI based vendors (Adoptium, Microsoft, ...) store it.
const HOME_VALUES: &[&str] = &["JavaHome", "InstallationPath", "Path"];

/// Depth of one registry walk, counted in keys below the scanned root. Adoptium
/// and Microsoft nest their home deepest: `<vendor>\JDK\<version>\hotspot\MSI`.
const MAX_REGISTRY_DEPTH: usize = 4;

/// Hard stop for one discovery run, so a machine with a pathological registry
/// cannot make the scan unbounded.
const MAX_REGISTRY_KEYS: usize = 1024;

/// Registry roots whose layout is known. Vendors that are not listed here are
/// found by name under `SOFTWARE` (see [`vendor_keys`]); `JavaSoft` and
/// `Microsoft` are excluded there because the explicit paths below cover them
/// and walking all of `SOFTWARE\Microsoft` would be both slow and useless.
const REGISTRY_ROOTS: &[&str] = &[
    r"SOFTWARE\JavaSoft\Java Development Kit",
    r"SOFTWARE\JavaSoft\Java Runtime Environment",
    r"SOFTWARE\JavaSoft\JRE",
    r"SOFTWARE\JavaSoft\JDK",
    r"SOFTWARE\JavaSoft\Java Plug-in",
    r"SOFTWARE\Eclipse Adoptium\JDK",
    r"SOFTWARE\Eclipse Adoptium\JRE",
    r"SOFTWARE\AdoptOpenJDK\JDK",
    r"SOFTWARE\AdoptOpenJDK\JRE",
    r"SOFTWARE\Microsoft\JDK",
    r"SOFTWARE\Azul Systems\Zulu",
    r"SOFTWARE\BellSoft\LibericaJDK",
    r"SOFTWARE\IBM\Semeru",
    r"SOFTWARE\IBM\Semeru Runtime",
];

/// `SOFTWARE` children that are handled explicitly and must never be walked
/// generically: they are large, mostly unrelated trees, and the explicit paths
/// above cover their Java keys.
const REGISTRY_SKIP_NAMES: &[&str] = &["ibm", "javasoft", "microsoft"];

/// Directory name fragments that identify a Java product. Matched as a
/// lowercase substring so version and vendor suffixes (`jdk-21.0.1+12`,
/// `zulu-17`, `LibericaJDK-21`) still count.
const JAVA_NAME_TOKENS: &[&str] = &[
    "java",
    "jdk",
    "jre",
    "openjdk",
    "temurin",
    "adoptium",
    "adoptopenjdk",
    "corretto",
    "zulu",
    "azul",
    "liberica",
    "bellsoft",
    "semeru",
    "graalvm",
    "sapmachine",
    "dragonwell",
    "mandrel",
    "ojdkbuild",
];

/// Extra top level directory names worth inspecting with [`is_java_like`]; the
/// JDK of these vendors sits inside a product directory
/// (`Program Files\Microsoft\jdk-...`, `Program Files\IBM\Semeru\...`).
const VENDOR_NAME_TOKENS: &[&str] = &["amazon", "eclipse", "ibm", "microsoft", "oracle", "sap"];

/// Grouping directories whose children are IDE products
/// (`Android\Android Studio`, `JetBrains\IntelliJ IDEA 2023.3`).
const IDE_GROUP_NAMES: &[&str] = &["android", "jetbrains"];

/// IDE product directory names.
const IDE_NAMES: &[&str] = &[
    "android studio",
    "intellij",
    "pycharm",
    "webstorm",
    "clion",
    "goland",
    "rider",
    "datagrip",
    "phpstorm",
    "rubymine",
    "dataspell",
    "rustrover",
    "aqua",
    "fleet",
    "mps",
    "appcode",
];

/// Runtime directories shipped inside an IDE: `jbr` is the JetBrains Runtime,
/// `jre64`/`jre` are the ones older IntelliJ and Android Studio releases used.
const IDE_RUNTIMES: &[&str] = &["jbr", "jbre", "jre", "jre64", "jre32"];

/// Maximum depth below a dedicated Java vendor or product directory.
const PRODUCT_DEPTH: usize = 3;

/// Depth for package manager packages (`scoop\apps\<package>\current\bin`).
const PACKAGE_DEPTH: usize = 3;

/// Depth for launcher managed runtimes
/// (`runtime\<runtime>\<platform>\<runtime>\bin`).
const RUNTIME_DEPTH: usize = 4;

/// Depth for the JetBrains Toolbox layout
/// (`apps\<product>\<channel>\<build>\jbr`).
const TOOLBOX_DEPTH: usize = 4;

// Re-exported by the parent discovery module.
pub(crate) fn discover() -> JavaDiscovery {
    let mut scanner = Scanner::new(None);
    scan_saved_config(&mut scanner);
    scan_environment(&mut scanner);
    scan_registry(&mut scanner);
    scan_filesystem(&mut scanner);
    scanner.finish()
}

/// The Java the user already picked in the launcher stays discoverable even if
/// it lives outside every known root.
fn scan_saved_config(scanner: &mut Scanner) {
    let Some(config) = crate::config::load_config() else {
        return;
    };
    if !config.java_path.is_empty() {
        scanner.candidate(PathBuf::from(config.java_path));
    }
}

fn scan_environment(scanner: &mut Scanner) {
    for variable in ["JAVA_HOME", "JRE_HOME", "JDK_HOME"] {
        let Some(home) = env_path(variable) else {
            continue;
        };
        scanner.home(&home);
        if !HOME_EXECUTABLES
            .iter()
            .any(|suffix| home.join(suffix).is_file())
        {
            scanner.warning(
                &home,
                format!("{variable} 未指向包含 Java 可执行文件的目录"),
            );
        }
    }
    if let Some(path) = std::env::var_os("PATH") {
        for directory in std::env::split_paths(&path) {
            scanner.candidate(directory.join("java.exe"));
            // Without a same-directory `java.exe` this stays a GUI candidate;
            // otherwise `Scanner::candidate` collapses it onto the console one.
            scanner.candidate(directory.join("javaw.exe"));
        }
    }
}

fn env_path(variable: &str) -> Option<PathBuf> {
    std::env::var_os(variable)
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
}

// ---------------------------------------------------------------------------
// Registry
// ---------------------------------------------------------------------------

#[derive(Clone, Copy, PartialEq, Eq)]
enum Root {
    LocalMachine,
    CurrentUser,
}

impl Root {
    fn label(self) -> &'static str {
        match self {
            Root::LocalMachine => "HKLM",
            Root::CurrentUser => "HKCU",
        }
    }

    fn key(self) -> RegKey {
        match self {
            Root::LocalMachine => RegKey::predef(HKEY_LOCAL_MACHINE),
            Root::CurrentUser => RegKey::predef(HKEY_CURRENT_USER),
        }
    }

    fn views(self) -> &'static [u32] {
        if self == Root::LocalMachine {
            // Also works for a 32-bit launcher on 64-bit Windows. WOW64 flags
            // are ignored by 32-bit Windows, where both views are identical.
            &[KEY_WOW64_64KEY, KEY_WOW64_32KEY]
        } else {
            &[0]
        }
    }
}

struct RegistryWalk {
    visited: usize,
    limit: usize,
}

impl Default for RegistryWalk {
    fn default() -> Self {
        Self {
            visited: 0,
            limit: MAX_REGISTRY_KEYS,
        }
    }
}

impl RegistryWalk {
    fn scan_root(&mut self, scanner: &mut Scanner, root: Root, path: &str) {
        for flags in root.views() {
            self.scan_key(scanner, root, path, *flags, MAX_REGISTRY_DEPTH);
        }
    }

    fn scan_key(
        &mut self,
        scanner: &mut Scanner,
        root: Root,
        path: &str,
        flags: u32,
        depth: usize,
    ) {
        if self.visited >= self.limit {
            scanner.warnings.insert(format!(
                "注册表扫描超过 {} 个键，已停止继续深入",
                self.limit
            ));
            return;
        }
        let key = match root.key().open_subkey_with_flags(path, KEY_READ | flags) {
            Ok(key) => key,
            Err(error) => {
                if let Some(message) = open_failure(&error) {
                    scanner.warning(&registry_path(root, path), message);
                }
                return;
            }
        };
        self.visited += 1;
        for name in HOME_VALUES {
            match key_home(&key, name) {
                Ok(Some(home)) => add_home(scanner, &home),
                Ok(None) => {}
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => scanner.warning(
                    &registry_path(root, path),
                    format!("无法读取注册表值 {name}: {error}"),
                ),
            }
        }
        if depth == 0 {
            return;
        }
        let mut children: Vec<String> = Vec::new();
        for child in key.enum_keys() {
            match child {
                Ok(name) => children.push(name),
                Err(error) => scanner.warning(
                    &registry_path(root, path),
                    format!("无法枚举注册表子项: {error}"),
                ),
            }
        }
        // Registry order is not stable; sorting keeps the scan (and the key
        // budget cutoff) deterministic.
        children.sort();
        for child in children {
            self.scan_key(scanner, root, &format!("{path}\\{child}"), flags, depth - 1);
        }
    }
}

/// A key that is simply not installed here is normal: most machines have only a
/// few of the known roots. Anything else, including missing permissions, is
/// worth telling the user about.
fn open_failure(error: &std::io::Error) -> Option<String> {
    (error.kind() != ErrorKind::NotFound).then(|| format!("无法读取注册表项: {error}"))
}

fn registry_path(root: Root, path: &str) -> PathBuf {
    PathBuf::from(format!("{}\\{path}", root.label()))
}

/// Registry homes are stored as `REG_SZ` or, when the installer wanted the path
/// to follow the shell (e.g. `%ProgramFiles%\Eclipse Adoptium`),
/// `REG_EXPAND_SZ`; only the latter is expanded, exactly like the shell does.
fn value_to_path(value: &RegValue) -> Option<PathBuf> {
    let text = match value.vtype {
        REG_SZ => String::from_reg_value(value).ok()?,
        REG_EXPAND_SZ => expand_environment(&String::from_reg_value(value).ok()?),
        _ => return None,
    };
    let text = text.trim();
    if text.is_empty() {
        return None;
    }
    Some(PathBuf::from(text))
}

fn key_home(key: &RegKey, name: &str) -> std::io::Result<Option<PathBuf>> {
    key.get_raw_value(name).map(|value| value_to_path(&value))
}

/// Installers point at the JDK root, at its `bin` directory, or occasionally at
/// the executable itself; accept all three instead of walking around.
fn add_home(scanner: &mut Scanner, home: &Path) {
    if home.is_file() {
        if home
            .file_name()
            .and_then(|name| name.to_str())
            .is_some_and(|name| {
                name.eq_ignore_ascii_case("java.exe") || name.eq_ignore_ascii_case("javaw.exe")
            })
        {
            scanner.candidate(home.to_path_buf());
        }
        return;
    }
    scanner.home(home);
    let is_bin = home
        .file_name()
        .is_some_and(|name| name.to_string_lossy().eq_ignore_ascii_case("bin"));
    if is_bin {
        scanner.candidate(home.join("java.exe"));
        scanner.candidate(home.join("javaw.exe"));
    }
}

fn scan_registry(scanner: &mut Scanner) {
    let mut walk = RegistryWalk::default();
    for root in [Root::LocalMachine, Root::CurrentUser] {
        for path in REGISTRY_ROOTS {
            walk.scan_root(scanner, root, path);
        }
        for path in vendor_keys(scanner, root, "SOFTWARE") {
            walk.scan_root(scanner, root, &path);
        }
    }
}

/// Vendors whose key layout is not spelled out in [`REGISTRY_ROOTS`] are found
/// by name, so new or renamed vendor keys still work. Only one level of `base`
/// is enumerated and only vendor names that also look Java related are walked;
/// the resulting paths are still subject to the depth and key budget.
fn vendor_keys(scanner: &mut Scanner, root: Root, base: &str) -> Vec<String> {
    let mut keys = Vec::new();
    for flags in root.views() {
        let key = match root.key().open_subkey_with_flags(base, KEY_READ | flags) {
            Ok(key) => key,
            Err(error) => {
                if let Some(message) = open_failure(&error) {
                    scanner.warning(&registry_path(root, base), message);
                }
                continue;
            }
        };
        for child in key.enum_keys() {
            match child {
                Ok(name) => {
                    let lower = name.to_ascii_lowercase();
                    if REGISTRY_SKIP_NAMES.contains(&lower.as_str()) || !is_vendor_name(&lower) {
                        continue;
                    }
                    keys.push(format!("{base}\\{name}"));
                }
                Err(error) => scanner.warning(
                    &registry_path(root, base),
                    format!("无法枚举注册表子项: {error}"),
                ),
            }
        }
    }
    keys.sort();
    keys.dedup();
    keys
}

// ---------------------------------------------------------------------------
// Filesystem
// ---------------------------------------------------------------------------

fn scan_filesystem(scanner: &mut Scanner) {
    for root in program_roots() {
        scan_program_files(scanner, &root);
    }
    if let Some(program_data) = env_path("ProgramData") {
        scan_program_data(scanner, &program_data);
    }
    if let Some(profile) = user_profile() {
        scan_profile(scanner, &profile);
    }
    if let Some(appdata) = env_path("APPDATA") {
        scan_appdata(scanner, &appdata);
    }
    if let Some(local) = env_path("LOCALAPPDATA") {
        scan_local_appdata(scanner, &local);
    }
    scan_custom_roots(scanner);
}

/// Application roots come from the environment, never from a hardcoded drive.
fn program_roots() -> Vec<PathBuf> {
    let mut roots = Vec::new();
    for variable in ["ProgramW6432", "ProgramFiles", "ProgramFiles(x86)"] {
        if let Some(root) = env_path(variable) {
            if !roots.contains(&root) {
                roots.push(root);
            }
        }
    }
    roots
}

fn user_profile() -> Option<PathBuf> {
    if let Some(profile) = env_path("USERPROFILE") {
        return Some(profile);
    }
    let mut profile = env_path("HOMEDRIVE")?.into_os_string();
    profile.push(env_path("HOMEPATH")?.as_os_str());
    Some(PathBuf::from(profile))
}

fn scan_program_files(scanner: &mut Scanner, root: &Path) {
    scan_application_root(scanner, root);
    // `javapath` holds per-JVM launcher links that resolve to the real binary.
    scanner.candidate(root.join("Common Files/Oracle/Java/javapath/java.exe"));
}

/// One `Program Files` shaped root. Only entries whose names are a Java vendor,
/// a JDK product, or an IDE that ships a JBR are inspected; the root itself is
/// never walked.
fn scan_application_root(scanner: &mut Scanner, root: &Path) {
    for entry in scanner.entries(root) {
        let Some(name) = directory_name(&entry) else {
            continue;
        };
        if IDE_GROUP_NAMES.contains(&name.as_str()) {
            // `Android\Android Studio`, `JetBrains\IntelliJ IDEA 2023.3`, ...
            for product in scanner.entries(&entry) {
                scan_ide(scanner, &product);
            }
        } else if is_ide_name(&name) {
            scan_ide(scanner, &entry);
        } else if is_java_like(&name) {
            scanner.tree(&entry, PRODUCT_DEPTH);
        } else if is_vendor_name(&name) {
            scan_vendor(scanner, &entry, PRODUCT_DEPTH);
        }
    }
}

/// Descends into the Java products of a vendor directory
/// (`Eclipse Adoptium\jdk-21.0.1.12-hotspot`, `Microsoft\jdk-17.0.5.8-hotspot`).
fn scan_vendor(scanner: &mut Scanner, vendor: &Path, depth: usize) {
    for product in scanner.entries(vendor) {
        let Some(name) = directory_name(&product) else {
            continue;
        };
        if is_java_like(&name) {
            scanner.tree(&product, depth);
        }
    }
}

/// IDEs ship their own runtime next to the product.
fn scan_ide(scanner: &mut Scanner, product: &Path) {
    for runtime in IDE_RUNTIMES {
        scanner.home(&product.join(runtime));
    }
}

fn scan_program_data(scanner: &mut Scanner, program_data: &Path) {
    scanner.candidate(program_data.join("Oracle/Java/javapath/java.exe"));
    scan_vendor(scanner, &program_data.join("scoop/apps"), PACKAGE_DEPTH);
    scan_vendor(scanner, &program_data.join("chocolatey/lib"), PACKAGE_DEPTH);
}

/// Toolchains that live in the user profile: IDE downloads, Gradle, version
/// managers and per-user package managers. Every entry is a fixed path, so no
/// part of the profile is walked.
fn scan_profile(scanner: &mut Scanner, profile: &Path) {
    for (suffix, depth) in [
        (".jdks", 2),
        (".gradle/jdks", 2),
        (".jabba/jdk", 2),
        (".jenv/versions", 1),
        (".sdkman/candidates/java", 2),
        (".asdf/installs/java", 2),
        (".local/share/mise/installs/java", 2),
    ] {
        scanner.tree(&profile.join(suffix), depth);
    }
    scan_vendor(scanner, &profile.join("scoop/apps"), PACKAGE_DEPTH);
}

/// Minecraft and its third party launchers keep downloaded runtimes in roaming
/// application data, a few levels below the launcher directory.
fn scan_appdata(scanner: &mut Scanner, appdata: &Path) {
    scanner.tree(&appdata.join(".minecraft/runtime"), RUNTIME_DEPTH);
    for launcher in ["PrismLauncher", "PolyMC"] {
        scanner.tree(&appdata.join(launcher).join("java"), 3);
        scanner.tree(&appdata.join(launcher).join("runtimes"), RUNTIME_DEPTH);
    }
}

/// Local application data holds per-user installs: MSI installers that were run
/// per-user, JetBrains Toolbox applications, mise, and per-user scoop.
fn scan_local_appdata(scanner: &mut Scanner, local: &Path) {
    scan_application_root(scanner, &local.join("Programs"));
    scanner.tree(&local.join("JetBrains/Toolbox/apps"), TOOLBOX_DEPTH);
    scanner.tree(&local.join("mise/installs/java"), 2);
    scan_vendor(scanner, &local.join("scoop/apps"), PACKAGE_DEPTH);
}

/// Version managers that let the user relocate their data directory.
fn scan_custom_roots(scanner: &mut Scanner) {
    for (variable, suffix) in [
        ("SDKMAN_DIR", "candidates/java"),
        ("SDKMAN_CANDIDATES_DIR", "java"),
        ("ASDF_DATA_DIR", "installs/java"),
        ("MISE_DATA_DIR", "installs/java"),
        ("MISE_INSTALLS_DIR", "java"),
        ("JABBA_HOME", "jdk"),
        ("JENV_ROOT", "versions"),
    ] {
        if let Some(root) = env_path(variable) {
            scanner.tree(&root.join(suffix), 2);
        }
    }
    for variable in ["SCOOP", "SCOOP_GLOBAL"] {
        if let Some(root) = env_path(variable) {
            scan_vendor(scanner, &root.join("apps"), PACKAGE_DEPTH);
        }
    }
}

/// Directory name of a directory entry, lowercased; `None` for files and for
/// names that cannot be matched against the token lists.
fn directory_name(entry: &Path) -> Option<String> {
    if !entry.is_dir() {
        return None;
    }
    Some(entry.file_name()?.to_str()?.to_ascii_lowercase())
}

fn is_java_like(name: &str) -> bool {
    JAVA_NAME_TOKENS.iter().any(|token| name.contains(token))
}

fn is_vendor_name(name: &str) -> bool {
    is_java_like(name)
        || VENDOR_NAME_TOKENS.iter().any(|token| {
            name.strip_prefix(token).is_some_and(|rest| {
                rest.is_empty() || rest.starts_with(' ') || rest.starts_with('-')
            })
        })
}

fn is_ide_name(name: &str) -> bool {
    IDE_NAMES.iter().any(|token| name.contains(token))
}

// ---------------------------------------------------------------------------
// Environment variable expansion
// ---------------------------------------------------------------------------

fn expand_environment(text: &str) -> String {
    expand_with(text, |name| std::env::var(name).ok())
}

/// `%NAME%` expansion with the shell's behavior: undefined names are kept
/// verbatim instead of being dropped, and `%%` is not a name.
fn expand_with(text: &str, lookup: impl Fn(&str) -> Option<String>) -> String {
    let mut expanded = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find('%') {
        let after = &rest[start + 1..];
        let Some(end) = after.find('%') else {
            break;
        };
        let name = &after[..end];
        match (name.is_empty(), lookup(name)) {
            (false, Some(value)) => {
                expanded.push_str(&rest[..start]);
                expanded.push_str(&value);
            }
            _ => expanded.push_str(&rest[..start + end + 2]),
        }
        rest = &after[end + 1..];
    }
    expanded.push_str(rest);
    expanded
}

#[cfg(test)]
mod tests {
    use std::sync::atomic::{AtomicU32, Ordering};

    use winreg::enums::{REG_DWORD, REG_EXPAND_SZ};
    use winreg::{RegKey, RegValue};

    use super::*;
    use crate::fixtures::{fixture, touch};

    /// Unique registry root under `HKCU`, deleted when the test ends. The real
    /// Java keys and the global environment are never touched.
    struct TempKey {
        path: String,
    }

    static TEMP_KEY_ID: AtomicU32 = AtomicU32::new(0);

    impl TempKey {
        fn new(tag: &str) -> Self {
            let path = format!(
                r"Software\NovaLauncherTests\{}-{}-{tag}",
                std::process::id(),
                TEMP_KEY_ID.fetch_add(1, Ordering::Relaxed)
            );
            // A leftover key from a crashed run is replaced, not reused.
            let hkcu = RegKey::predef(HKEY_CURRENT_USER);
            let _ = hkcu.delete_subkey_all(&path);
            hkcu.create_subkey(&path).unwrap();
            Self { path }
        }

        fn create(&self, suffix: &str) -> RegKey {
            RegKey::predef(HKEY_CURRENT_USER)
                .create_subkey(format!(r"{}\{suffix}", self.path))
                .unwrap()
                .0
        }
    }

    impl Drop for TempKey {
        fn drop(&mut self) {
            let _ = RegKey::predef(HKEY_CURRENT_USER).delete_subkey_all(&self.path);
        }
    }

    fn expand_sz(text: &str) -> RegValue {
        let mut bytes: Vec<u8> = text.encode_utf16().flat_map(u16::to_le_bytes).collect();
        bytes.extend_from_slice(&[0, 0]);
        RegValue {
            bytes,
            vtype: REG_EXPAND_SZ,
        }
    }

    /// Creates `<jdk>/bin/java.exe` and returns the registry home for it.
    fn jdk(root: &Path, name: &str) -> (PathBuf, String) {
        let home = root.join(name);
        touch(&home.join("bin/java.exe"));
        let text = home.display().to_string();
        (home, text)
    }

    /// Candidates are canonicalized, so a whole path cannot be compared
    /// directly: `\\?\` prefixes and the temp directory spelling vary.
    fn has_candidate(scanner: &Scanner, suffix: &str) -> bool {
        let suffix = suffix.replace('/', "\\").to_ascii_lowercase();
        scanner.candidates.iter().any(|path| {
            path.to_string_lossy()
                .trim_start_matches(r"\\?\")
                .to_ascii_lowercase()
                .ends_with(&suffix)
        })
    }

    fn scan_roots(scanner: &mut Scanner, temp: &TempKey) -> RegistryWalk {
        let mut walk = RegistryWalk::default();
        for path in REGISTRY_ROOTS {
            walk.scan_root(
                scanner,
                Root::CurrentUser,
                &format!(r"{}\{path}", temp.path),
            );
        }
        walk
    }

    #[test]
    fn registry_homes_are_read_from_every_value_name_and_depth() {
        let root = fixture("java-windows-registry-values");
        let temp = TempKey::new("values");
        let (_, spaced) = jdk(&root, "Nova Test JDK");
        let (_, unicode) = jdk(&root, "Zulu 安装 运行环境");
        let (_, adoptium) = jdk(&root, "Adoptium");

        temp.create(r"SOFTWARE\JavaSoft\Java Development Kit\21.0.1")
            .set_value("JavaHome", &spaced)
            .unwrap();
        temp.create(r"SOFTWARE\Azul Systems\Zulu\17.0.9")
            .set_value("InstallationPath", &unicode)
            .unwrap();
        temp.create(r"SOFTWARE\Eclipse Adoptium\JDK\21.0.1.12-hotspot\hotspot\MSI")
            .set_value("Path", &adoptium)
            .unwrap();

        let mut scanner = Scanner::new(None);
        let walk = scan_roots(&mut scanner, &temp);
        assert_eq!(scanner.candidates.len(), 3);
        assert!(has_candidate(&scanner, r"Nova Test JDK\bin\java.exe"));
        assert!(has_candidate(&scanner, r"Zulu 安装 运行环境\bin\java.exe"));
        assert!(has_candidate(&scanner, r"Adoptium\bin\java.exe"));
        assert!(scanner.warnings.is_empty());
        assert!(walk.visited > 0);
    }

    #[test]
    fn registry_depth_bound_ignores_keys_below_the_limit() {
        let root = fixture("java-windows-registry-depth");
        let temp = TempKey::new("depth");
        let (_, shallow) = jdk(&root, "Shallow JDK");
        let (_, deep) = jdk(&root, "Too Deep JDK");
        temp.create(r"a\b\c\d")
            .set_value("JavaHome", &shallow)
            .unwrap();
        temp.create(r"a\b\c\d\e")
            .set_value("JavaHome", &deep)
            .unwrap();

        let mut scanner = Scanner::new(None);
        let mut walk = RegistryWalk::default();
        walk.scan_root(&mut scanner, Root::CurrentUser, &temp.path);
        // The scanned root plus four levels below it: nothing more is opened.
        assert_eq!(walk.visited, 5);
        assert!(has_candidate(&scanner, r"Shallow JDK\bin\java.exe"));
        assert!(!has_candidate(&scanner, r"Too Deep JDK\bin\java.exe"));
        assert_eq!(scanner.candidates.len(), 1);
    }

    #[test]
    fn registry_key_budget_stops_scanning_and_warns() {
        let temp = TempKey::new("budget");
        for index in 0..5 {
            temp.create(&format!("k{index}"));
        }
        let mut scanner = Scanner::new(None);
        let mut walk = RegistryWalk {
            visited: 0,
            limit: 2,
        };
        walk.scan_root(&mut scanner, Root::CurrentUser, &temp.path);
        assert_eq!(walk.visited, 2);
        assert_eq!(scanner.warnings.len(), 1);
    }

    #[test]
    fn absent_registry_roots_are_silent() {
        let mut scanner = Scanner::new(None);
        let mut walk = RegistryWalk::default();
        walk.scan_root(
            &mut scanner,
            Root::CurrentUser,
            r"Software\NovaLauncherTests\does-not-exist",
        );
        walk.scan_root(
            &mut scanner,
            Root::LocalMachine,
            r"SOFTWARE\NovaLauncherTests\does-not-exist",
        );
        assert_eq!(walk.visited, 0);
        assert!(scanner.candidates.is_empty());
        assert!(scanner.warnings.is_empty());
    }

    #[test]
    fn registry_roots_read_msi_paths_and_never_walk_microsoft_generically() {
        let root = fixture("java-windows-registry-microsoft");
        let temp = TempKey::new("microsoft");
        let (_, microsoft) = jdk(&root, "Microsoft JDK");
        let (_, edge) = jdk(&root, "Edge JDK");
        temp.create(r"SOFTWARE\Microsoft\JDK\21.0.2\hotspot\MSI")
            .set_value("Path", &microsoft)
            .unwrap();
        // Same value names, but outside `Microsoft\JDK`: must stay invisible.
        temp.create(r"SOFTWARE\Microsoft\Edge\Update")
            .set_value("Path", &edge)
            .unwrap();

        let mut scanner = Scanner::new(None);
        scan_roots(&mut scanner, &temp);
        assert!(has_candidate(&scanner, r"Microsoft JDK\bin\java.exe"));
        assert!(!has_candidate(&scanner, r"Edge JDK\bin\java.exe"));
        assert_eq!(scanner.candidates.len(), 1);
    }

    #[test]
    fn vendor_keys_find_java_vendors_only() {
        let root = fixture("java-windows-registry-vendors");
        let temp = TempKey::new("vendors");
        let (_, dragonwell) = jdk(&root, "Dragonwell JDK");
        temp.create(r"SOFTWARE\Dragonwell LLC\jdk-21.0.2")
            .set_value("JavaHome", &dragonwell)
            .unwrap();
        temp.create(r"SOFTWARE\Microsoft\Edge")
            .set_value("Path", &dragonwell)
            .unwrap();
        temp.create(r"SOFTWARE\Unrelated Software")
            .set_value("Path", &dragonwell)
            .unwrap();
        temp.create(r"SOFTWARE\JavaSoft\Prefs")
            .set_value("Path", &dragonwell)
            .unwrap();
        temp.create(r"SOFTWARE\IBM\SPSS")
            .set_value("Path", &dragonwell)
            .unwrap();

        let software = format!(r"{}\SOFTWARE", temp.path);
        let mut scanner = Scanner::new(None);
        let keys = vendor_keys(&mut scanner, Root::CurrentUser, &software);
        assert_eq!(keys, vec![format!(r"{software}\Dragonwell LLC")]);
        for key in keys {
            let mut walk = RegistryWalk::default();
            walk.scan_root(&mut scanner, Root::CurrentUser, &key);
        }
        assert!(has_candidate(&scanner, r"Dragonwell JDK\bin\java.exe"));
        assert_eq!(scanner.candidates.len(), 1);
        assert!(scanner.warnings.is_empty());
    }

    #[test]
    fn expand_sz_homes_expand_environment_variables() {
        let temp = TempKey::new("expand");
        let key = temp.create("jdk");
        let system_root = std::env::var("SystemRoot").expect("Windows always sets SystemRoot");

        key.set_raw_value(
            "JavaHome",
            &expand_sz(r"%SystemRoot%\Nova Test JDK\bin\java.exe"),
        )
        .unwrap();
        let value = key.get_raw_value("JavaHome").unwrap();
        assert_eq!(value.vtype, REG_EXPAND_SZ);
        assert_eq!(
            value_to_path(&value).unwrap(),
            PathBuf::from(format!(r"{system_root}\Nova Test JDK\bin\java.exe"))
        );

        // `REG_SZ` is used verbatim, even when it contains a `%` name.
        key.set_value("Path", &r"%SystemRoot%\Nova Test JDK")
            .unwrap();
        assert_eq!(
            value_to_path(&key.get_raw_value("Path").unwrap()).unwrap(),
            PathBuf::from(r"%SystemRoot%\Nova Test JDK")
        );

        // A home is a string; anything else is ignored without a warning.
        key.set_value("InstallationPath", &21u32).unwrap();
        let value = key.get_raw_value("InstallationPath").unwrap();
        assert_eq!(value.vtype, REG_DWORD);
        assert!(value_to_path(&value).is_none());
        key.set_value("JavaHome", &"").unwrap();
        assert!(value_to_path(&key.get_raw_value("JavaHome").unwrap()).is_none());
        assert!(key.get_raw_value("Absent").is_err());
    }

    #[test]
    fn environment_expansion_keeps_unknown_names_literal() {
        let lookup = |name: &str| match name {
            "JDK" => Some(r"C:\Program Files\Eclipse Adoptium\jdk-21".to_string()),
            "EMPTY" => Some(String::new()),
            _ => None,
        };
        assert_eq!(
            expand_with(r"%JDK%\bin\java.exe", lookup),
            r"C:\Program Files\Eclipse Adoptium\jdk-21\bin\java.exe"
        );
        assert_eq!(expand_with(r"%%", lookup), "%%");
        assert_eq!(expand_with("50%", lookup), "50%");
        assert_eq!(
            expand_with(r"%JDK%\%MISSING%\x", lookup),
            r"C:\Program Files\Eclipse Adoptium\jdk-21\%MISSING%\x"
        );
        assert_eq!(expand_with("%EMPTY%tail", lookup), "tail");
    }

    #[test]
    fn application_roots_only_inspect_java_vendor_and_ide_directories() {
        let root = fixture("java-windows-roots");
        let programs = root.join("Program Files");
        touch(&programs.join("Java/jdk-21.0.1/bin/java.exe"));
        touch(&programs.join("Eclipse Adoptium/jdk-21.0.1.12-hotspot/bin/java.exe"));
        touch(&programs.join("Java 安装 JDK/jre/bin/java.exe"));
        touch(&programs.join("Microsoft/jdk-17.0.5.8-hotspot/bin/java.exe"));
        touch(&programs.join("Temurin-21/bin/java.exe"));
        touch(&programs.join("Amazon Corretto/21.0.1/bin/java.exe"));
        touch(&programs.join("Microsoft/Edge/bin/java.exe"));
        touch(&programs.join("JetBrains/IntelliJ IDEA 2023.3/jbr/bin/java.exe"));
        touch(&programs.join("Android/Android Studio/jbr/bin/java.exe"));
        touch(&programs.join("Common Files/Oracle/Java/javapath/java.exe"));
        touch(&programs.join("Unrelated Tools/jdk-21/bin/java.exe"));
        touch(&programs.join("Unrelated/Java/bin/java.exe"));
        touch(&programs.join("WindowsApps/jdk-hidden/bin/java.exe"));

        let mut scanner = Scanner::new(None);
        scan_program_files(&mut scanner, &programs);
        assert_eq!(scanner.candidates.len(), 9);
        assert!(has_candidate(&scanner, r"Java\jdk-21.0.1\bin\java.exe"));
        assert!(has_candidate(
            &scanner,
            r"Eclipse Adoptium\jdk-21.0.1.12-hotspot\bin\java.exe"
        ));
        assert!(has_candidate(&scanner, r"Java 安装 JDK\jre\bin\java.exe"));
        assert!(has_candidate(
            &scanner,
            r"Microsoft\jdk-17.0.5.8-hotspot\bin\java.exe"
        ));
        assert!(has_candidate(&scanner, r"Temurin-21\bin\java.exe"));
        assert!(has_candidate(
            &scanner,
            r"Amazon Corretto\21.0.1\bin\java.exe"
        ));
        assert!(has_candidate(
            &scanner,
            r"JetBrains\IntelliJ IDEA 2023.3\jbr\bin\java.exe"
        ));
        assert!(has_candidate(&scanner, r"Android Studio\jbr\bin\java.exe"));
        assert!(!has_candidate(&scanner, r"Microsoft\Edge\bin\java.exe"));
        assert!(!has_candidate(
            &scanner,
            r"Unrelated Tools\jdk-21\bin\java.exe"
        ));
        assert!(!has_candidate(&scanner, r"Unrelated\Java\bin\java.exe"));
        assert!(!has_candidate(&scanner, r"WindowsApps\jdk-hidden\bin\java.exe"));
        assert!(scanner.warnings.is_empty());
    }

    #[test]
    fn package_manager_roots_match_java_packages_only() {
        let root = fixture("java-windows-packages");
        let apps = root.join("apps");
        touch(&apps.join("openjdk/current/bin/java.exe"));
        touch(&apps.join("temurin-lts/21.0.1/bin/java.exe"));
        touch(&apps.join("notepadplusplus/current/bin/java.exe"));
        touch(&apps.join("keepass/bin/java.exe"));

        let mut scanner = Scanner::new(None);
        scan_vendor(&mut scanner, &apps, PACKAGE_DEPTH);
        // The `current` level is a junction in a real scoop installation, so the
        // candidate resolves through it; here it is a plain directory.
        assert_eq!(scanner.candidates.len(), 2);
        assert!(has_candidate(&scanner, r"openjdk\current\bin\java.exe"));
        assert!(has_candidate(&scanner, r"temurin-lts\21.0.1\bin\java.exe"));
        assert!(!has_candidate(
            &scanner,
            r"notepadplusplus\current\bin\java.exe"
        ));
        assert!(!has_candidate(&scanner, r"keepass\bin\java.exe"));
        assert!(scanner.warnings.is_empty());
    }

    #[test]
    fn user_profile_and_launcher_data_roots_are_bounded() {
        let root = fixture("java-windows-user");
        let profile = root.join("profile");
        touch(&profile.join(".jdks/jbr-17.0.6/bin/java.exe"));
        touch(&profile.join(".gradle/jdks/eclipse_adoptium-21-amd64-windows/bin/java.exe"));
        touch(&profile.join(".jabba/jdk/openjdk@1.17.0/bin/java.exe"));
        touch(&profile.join(".jenv/versions/17.0/bin/java.exe"));
        touch(&profile.join(".sdkman/candidates/java/21.0.1-tem/bin/java.exe"));
        touch(&profile.join(".asdf/installs/java/temurin-17.0.6/bin/java.exe"));
        touch(&profile.join("scoop/apps/corretto17/current/bin/java.exe"));
        touch(&profile.join(".toolchains/jdk-21/bin/java.exe"));
        touch(&profile.join(".jdks/jbr-17.0.6/jbr-17.0.6/deeper/bin/java.exe"));

        let appdata = root.join("appdata");
        touch(&appdata.join(
            ".minecraft/runtime/java-runtime-gamma/windows-x64/java-runtime-gamma/bin/java.exe",
        ));
        touch(&appdata.join("PrismLauncher/java/temurin-17/bin/java.exe"));
        touch(&appdata.join("PolyMC/runtimes/java-runtime-delta/windows-x64/jre/bin/java.exe"));

        let local = root.join("local");
        touch(&local.join("Programs/Eclipse Adoptium/jdk-21.0.1.12-hotspot/bin/java.exe"));
        touch(&local.join("Programs/NotAnIde/jbr/bin/java.exe"));
        touch(&local.join(
            "JetBrains/Toolbox/apps/IntelliJ IDEA Ultimate/ch-0/231.5920.6/jbr/bin/java.exe",
        ));

        let mut scanner = Scanner::new(None);
        scan_profile(&mut scanner, &profile);
        scan_appdata(&mut scanner, &appdata);
        scan_local_appdata(&mut scanner, &local);

        let expected = [
            r".jdks\jbr-17.0.6\bin\java.exe",
            r".gradle\jdks\eclipse_adoptium-21-amd64-windows\bin\java.exe",
            r".jabba\jdk\openjdk@1.17.0\bin\java.exe",
            r".jenv\versions\17.0\bin\java.exe",
            r".sdkman\candidates\java\21.0.1-tem\bin\java.exe",
            r".asdf\installs\java\temurin-17.0.6\bin\java.exe",
            r"scoop\apps\corretto17\current\bin\java.exe",
            r".minecraft\runtime\java-runtime-gamma\windows-x64\java-runtime-gamma\bin\java.exe",
            r"PrismLauncher\java\temurin-17\bin\java.exe",
            r"PolyMC\runtimes\java-runtime-delta\windows-x64\jre\bin\java.exe",
            r"Programs\Eclipse Adoptium\jdk-21.0.1.12-hotspot\bin\java.exe",
            r"Toolbox\apps\IntelliJ IDEA Ultimate\ch-0\231.5920.6\jbr\bin\java.exe",
        ];
        assert_eq!(scanner.candidates.len(), expected.len());
        for suffix in expected {
            assert!(has_candidate(&scanner, suffix), "missing {suffix}");
        }
        assert!(!has_candidate(&scanner, r".toolchains\jdk-21\bin\java.exe"));
        assert!(!has_candidate(
            &scanner,
            r"Programs\NotAnIde\jbr\bin\java.exe"
        ));
        assert!(!has_candidate(
            &scanner,
            r"jbr-17.0.6\jbr-17.0.6\deeper\bin\java.exe"
        ));
        assert!(scanner.warnings.is_empty());
    }
}
