//! Detection rules, loaded from a JSON file so new distributions can be added without
//! rebuilding the binary.

use std::collections::HashMap;
use std::path::Path;

use anyhow::{Context, Result};
use regex::Regex;
use serde::{Deserialize, Deserializer};

/// Rules shipped with the binary, used if no rules file is installed.
pub const BUILTIN: &str = include_str!("../rules.json");

/// Rules installed with the package.
pub const RULES_FILE: &str = "/usr/share/pxvirt-isoinfo/rules.json";

/// Local rules, merged in alphabetical order and checked before the packaged rules.
pub const RULES_DIR: &str = "/etc/pxvirt-isoinfo/rules.d";

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Rules {
    /// map architecture names to a common name, e.g. amd64 -> x86_64
    #[serde(default)]
    pub arch_aliases: HashMap<String, String>,
    pub windows: WindowsRules,
    #[serde(default)]
    pub linux: Vec<LinuxRule>,
}

/// A local rules file, everything is optional.
#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalRules {
    #[serde(default)]
    arch_aliases: HashMap<String, String>,
    #[serde(default)]
    windows: LocalWindowsRules,
    #[serde(default)]
    linux: Vec<LinuxRule>,
}

#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
struct LocalWindowsRules {
    #[serde(default)]
    images: Vec<String>,
    #[serde(default)]
    versions: Vec<WindowsVersion>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsRules {
    /// WIM/ESD files holding the installable images, the first existing one is used
    pub images: Vec<String>,
    #[serde(default)]
    pub installer: Option<String>,
    /// first matching entry wins
    #[serde(default)]
    pub versions: Vec<WindowsVersion>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WindowsVersion {
    #[serde(default)]
    pub installation_type: Option<Pattern>,
    #[serde(default)]
    pub min_build: Option<u32>,
    #[serde(default)]
    pub max_build: Option<u32>,
    pub name: String,
    #[serde(default)]
    pub ostype: Option<String>,
}

impl WindowsVersion {
    pub fn matches(&self, installation_type: &str, build: u32) -> bool {
        self.installation_type.as_ref().is_none_or(|re| re.0.is_match(installation_type))
            && self.min_build.is_none_or(|min| build >= min)
            && self.max_build.is_none_or(|max| build <= max)
    }
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct LinuxRule {
    pub id: String,
    pub name: String,
    #[serde(default)]
    pub installer: Option<String>,
    #[serde(default = "default_linux_ostype")]
    pub ostype: Option<String>,
    #[serde(rename = "match")]
    pub matches: Match,
    /// extract name, version and arch with named capture groups, the first match of each
    /// group wins
    #[serde(default)]
    pub info: Vec<ContentRule>,
}

fn default_linux_ostype() -> Option<String> {
    Some("l26".to_string())
}

/// All given conditions must be fulfilled.
#[derive(Default, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct Match {
    /// volume label
    #[serde(default)]
    pub label: Option<Pattern>,
    /// all of these paths must exist
    #[serde(default)]
    pub files_all: Vec<String>,
    /// at least one of these paths must exist
    #[serde(default)]
    pub files_any: Vec<String>,
    /// all of these files must exist and match
    #[serde(default)]
    pub content: Vec<ContentRule>,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ContentRule {
    /// path in the image, or "@label" for the volume label
    pub file: String,
    pub regex: Pattern,
}

pub struct Pattern(pub Regex);

impl<'de> Deserialize<'de> for Pattern {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let pattern = String::deserialize(deserializer)?;
        Regex::new(&pattern).map(Pattern).map_err(serde::de::Error::custom)
    }
}

impl Rules {
    pub fn parse(data: &str) -> Result<Self> {
        Ok(serde_json::from_str(data)?)
    }

    /// Load the given rules file, or the packaged rules (falling back to the built-in ones)
    /// extended by the local rules.
    pub fn load(path: Option<&Path>) -> Result<Self> {
        if let Some(path) = path {
            return Self::parse(&read(path)?).with_context(|| format!("invalid rules in {path:?}"));
        }

        let path = Path::new(RULES_FILE);
        let mut rules = if path.exists() {
            Self::parse(&read(path)?).with_context(|| format!("invalid rules in {path:?}"))?
        } else {
            Self::parse(BUILTIN).context("invalid built-in rules")?
        };

        let mut local = match std::fs::read_dir(RULES_DIR) {
            Ok(dir) => dir
                .filter_map(|entry| entry.ok().map(|e| e.path()))
                .filter(|path| path.extension().is_some_and(|ext| ext == "json"))
                .collect::<Vec<_>>(),
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => Vec::new(),
            Err(err) => return Err(err).context(format!("unable to read {RULES_DIR}")),
        };
        local.sort();
        // merge in reverse, so the first file ends up first
        for path in local.iter().rev() {
            let extra: LocalRules =
                serde_json::from_str(&read(path)?).with_context(|| format!("invalid rules in {path:?}"))?;
            rules.extend(extra);
        }

        Ok(rules)
    }

    /// Add rules, which take precedence over the existing ones.
    fn extend(&mut self, extra: LocalRules) {
        self.arch_aliases.extend(extra.arch_aliases);
        self.windows.images.splice(0..0, extra.windows.images);
        self.windows.versions.splice(0..0, extra.windows.versions);
        self.linux.splice(0..0, extra.linux);
    }

    pub fn arch(&self, arch: &str) -> String {
        self.arch_aliases.get(arch).cloned().unwrap_or_else(|| arch.to_string())
    }
}

fn read(path: &Path) -> Result<String> {
    std::fs::read_to_string(path).with_context(|| format!("unable to read {path:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builtin() {
        let rules = Rules::parse(BUILTIN).unwrap();
        assert!(!rules.linux.is_empty());
        assert_eq!(rules.arch("amd64"), "x86_64");
    }

    #[test]
    fn extend() {
        let mut rules = Rules::parse(BUILTIN).unwrap();
        let count = rules.linux.len();
        let extra: LocalRules = serde_json::from_str(
            r#"{"linux": [{"id": "test", "name": "Test", "match": {"files_all": ["test"]}}],
                "arch_aliases": {"aarch64": "arm64"}}"#,
        )
        .unwrap();
        rules.extend(extra);
        assert_eq!(rules.linux.len(), count + 1);
        assert_eq!(rules.linux[0].id, "test");
        assert_eq!(rules.linux[0].ostype.as_deref(), Some("l26"));
        assert_eq!(rules.arch("aarch64"), "arm64");
        assert!(!rules.windows.versions.is_empty());
    }
}
